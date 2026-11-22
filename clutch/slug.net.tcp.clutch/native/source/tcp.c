#if defined(_WIN32)
/* WSAPoll and its pollfd need Windows Vista's API. */
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0600
#endif
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#else
/* POSIX sockets; shims build with -std=c11. */
#define _POSIX_C_SOURCE 200809L
#if defined(__APPLE__)
#define _DARWIN_C_SOURCE
#endif
#if defined(__linux__)
#define _GNU_SOURCE
#endif
#endif

#include "slug_net_tcp.h"

#include <errno.h>
#include <limits.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if !defined(_WIN32)
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#if defined(__APPLE__) || defined(__FreeBSD__) || defined(__NetBSD__) || defined(__OpenBSD__)
#define SLUG_TCP_KQUEUE 1
#include <sys/event.h>
#elif defined(__linux__)
#define SLUG_TCP_EPOLL 1
#include <sys/epoll.h>
#else
#error "slug.net.tcp needs kqueue, epoll, or Winsock"
#endif
#endif

/* Bytes a connection receives at once, and so the largest chunk it sends. */
#define READ_BUFFER 16384
/* How long the peer may take nothing before a write times out. */
#define DEFAULT_WRITE_TIMEOUT_MS 5000
/* Bytes sent to a connection at once. */
#define WRITE_SLICE 65536
/* Kernel events taken at once. */
#define EVENTS 256

/* --- The system: sockets, errors, locks, threads ---------------------------- */

#if defined(_WIN32)

typedef SOCKET socket_t;
#define NO_SOCKET INVALID_SOCKET
#define SHUT_WR SD_SEND
#define SHUT_RDWR SD_BOTH

/* Winsock's numbers for the errors the code tells apart; ENOMEM and EINVAL
 * are the C library's, as on POSIX. */
#define NET_EINTR WSAEINTR
#define NET_EINPROGRESS WSAEWOULDBLOCK
#define NET_ECONNREFUSED WSAECONNREFUSED
#define NET_ETIMEDOUT WSAETIMEDOUT
#define NET_EPIPE WSAESHUTDOWN

static int socket_error(void) { return WSAGetLastError(); }
static bool would_block(int error) { return error == WSAEWOULDBLOCK; }
static void close_socket(socket_t socket) { closesocket(socket); }

typedef SRWLOCK mutex_t;
#define MUTEX_INITIALIZER SRWLOCK_INIT
static void mutex_init(mutex_t *mutex) { InitializeSRWLock(mutex); }
static void mutex_lock(mutex_t *mutex) { AcquireSRWLockExclusive(mutex); }
static void mutex_unlock(mutex_t *mutex) { ReleaseSRWLockExclusive(mutex); }
static void mutex_destroy(mutex_t *mutex) { (void)mutex; }

typedef HANDLE thread_t;
#define THREAD_FUNCTION(name) static DWORD WINAPI name(LPVOID argument)
#define THREAD_RETURN return 0
typedef DWORD(WINAPI *thread_body)(LPVOID);

/* Starts `body`, returning 0 or the error it failed with. */
static int thread_start(thread_t *thread, thread_body body, void *argument) {
  *thread = CreateThread(NULL, 0, body, argument, 0, NULL);
  return *thread == NULL ? (int)GetLastError() : 0;
}
static void thread_join(thread_t thread) {
  WaitForSingleObject(thread, INFINITE);
  CloseHandle(thread);
}
static void thread_detach(thread_t thread) { CloseHandle(thread); }

static void pause_briefly(void) { Sleep(1); }
static int64_t now_ms(void) { return (int64_t)GetTickCount64(); }

static void set_close_on_exec(socket_t socket) {
  SetHandleInformation((HANDLE)socket, HANDLE_FLAG_INHERIT, 0);
}

static void set_blocking(socket_t socket, bool blocking) {
  u_long nonblocking = blocking ? 0 : 1;
  ioctlsocket(socket, (long)FIONBIO, &nonblocking);
}

/* Winsock, started once for the process: 0, or the error it failed with. */
static INIT_ONCE network_once = INIT_ONCE_STATIC_INIT;
static int network_error = 0;
static BOOL CALLBACK start_network(PINIT_ONCE once, PVOID parameter, PVOID *context) {
  WSADATA data;
  (void)once;
  (void)parameter;
  (void)context;
  network_error = WSAStartup(MAKEWORD(2, 2), &data);
  return TRUE;
}
static int ensure_network(void) {
  InitOnceExecuteOnce(&network_once, start_network, NULL, NULL);
  return network_error;
}

static int64_t socket_send(socket_t socket, const uint8_t *data, size_t length) {
  return send(socket, (const char *)data, (int)length, 0);
}
static int64_t socket_receive(socket_t socket, uint8_t *buffer, size_t length) {
  return recv(socket, (char *)buffer, (int)length, 0);
}

#else

typedef int socket_t;
#define NO_SOCKET (-1)

#define NET_EINTR EINTR
#define NET_EINPROGRESS EINPROGRESS
#define NET_ECONNREFUSED ECONNREFUSED
#define NET_ETIMEDOUT ETIMEDOUT
#define NET_EPIPE EPIPE

static int socket_error(void) { return errno; }
static bool would_block(int error) { return error == EAGAIN || error == EWOULDBLOCK; }
static void close_socket(socket_t socket) { close(socket); }

typedef pthread_mutex_t mutex_t;
#define MUTEX_INITIALIZER PTHREAD_MUTEX_INITIALIZER
static void mutex_init(mutex_t *mutex) { pthread_mutex_init(mutex, NULL); }
static void mutex_lock(mutex_t *mutex) { pthread_mutex_lock(mutex); }
static void mutex_unlock(mutex_t *mutex) { pthread_mutex_unlock(mutex); }
static void mutex_destroy(mutex_t *mutex) { pthread_mutex_destroy(mutex); }

typedef pthread_t thread_t;
#define THREAD_FUNCTION(name) static void *name(void *argument)
#define THREAD_RETURN return NULL
typedef void *(*thread_body)(void *);

static int thread_start(thread_t *thread, thread_body body, void *argument) {
  return pthread_create(thread, NULL, body, argument);
}
static void thread_join(thread_t thread) { pthread_join(thread, NULL); }
static void thread_detach(thread_t thread) { pthread_detach(thread); }

static void pause_briefly(void) {
  struct timespec delay = {0, 1000000};
  nanosleep(&delay, NULL);
}

static int64_t now_ms(void) {
  struct timespec now;
  clock_gettime(CLOCK_MONOTONIC, &now);
  return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static void set_close_on_exec(socket_t fd) { fcntl(fd, F_SETFD, fcntl(fd, F_GETFD) | FD_CLOEXEC); }

static void set_blocking(socket_t fd, bool blocking) {
  int flags = fcntl(fd, F_GETFL);
  fcntl(fd, F_SETFL, blocking ? flags & ~O_NONBLOCK : flags | O_NONBLOCK);
}

static int ensure_network(void) { return 0; }

static int64_t socket_send(socket_t fd, const uint8_t *data, size_t length) {
  int flags = 0;
#if defined(MSG_NOSIGNAL)
  flags = MSG_NOSIGNAL;
#endif
  return (int64_t)send(fd, data, length, flags);
}
static int64_t socket_receive(socket_t fd, uint8_t *buffer, size_t length) {
  return (int64_t)recv(fd, buffer, length, 0);
}

#endif

/* Every connection's reading and writing happens on one thread, the I/O
 * thread, which waits on all their sockets at once (kqueue, epoll, or
 * WSAPoll) and starts with the first connection. Threads that write queue
 * the bytes and hand the connection to the I/O thread, which sends what the
 * peer takes; what arrives it sends through the connection's `chunks`
 * producer. A thread or two per connection would hold a stack each, and
 * memory the allocator keeps per thread, for as long as the connection is
 * open. */

/* Bytes waiting to be sent, oldest first. */
struct pending {
  struct pending *next;
  size_t length;
  size_t sent;
  uint8_t data[];
};

struct slug_tcp_connection {
  socket_t fd;
  /* The resource holds one reference, the I/O thread one from `start` until
   * it has finished with the connection, and the I/O thread's list of
   * connections to look at one while it is on it. The socket closes when
   * the last goes. */
  atomic_int references;
  atomic_bool closed;
  atomic_bool started;
  char peer[64];

  /* Guarded by `lock`. */
  mutex_t lock;
  struct pending *first;
  struct pending *last;
  size_t queued;
  /* The error a failed send ended writing with, or 0. */
  int write_error;
  /* Set by `slug_tcp_wait_writable`: the I/O thread sends a tick through
   * `writable` when the queue next shrinks. */
  bool waiting;
  /* From `start` until the I/O thread has finished writing. */
  bool writing;
  /* The peer took nothing of the last send: the I/O thread sends the rest
   * once the socket can take more, and needs no word of later writes. */
  bool blocked;
  /* `slug_tcp_shutdown_write` was called; `shut_down` once it is done. */
  bool shutdown_requested;
  bool shut_down;
  int64_t write_timeout_ms;
  slug_producer *writable;

  /* Guarded by the I/O thread's lock. */
  bool changed;
  struct slug_tcp_connection *next_changed;

  /* The I/O thread's own. */
  slug_producer *chunks;
  bool registered;
  bool reading;
  bool want_read;
  bool want_write;
  bool finished;
  /* Bytes the chunk channel had no room for, or with `held_error` an error
   * number, sent again once it does; meanwhile the socket is not read. */
  uint8_t *held;
  size_t held_length;
  int held_error;
  bool holding;
  /* When the peer last took bytes, while the I/O thread waits to send. */
  int64_t progress_ms;
  /* Doubly linked lists of the connections holding a chunk, and of those
   * waiting to send. */
  struct slug_tcp_connection *prev_held, *next_held;
  struct slug_tcp_connection *prev_blocked, *next_blocked;
  bool on_held, on_blocked;
  /* Finished connections whose reference the I/O thread drops at the end of
   * a round, after any events it took for them in it. */
  struct slug_tcp_connection *next_retired;
#if defined(_WIN32)
  /* Its place in the I/O thread's list of the sockets it polls. */
  size_t watch_index;
#endif
};

struct slug_tcp_listener {
  socket_t fd;
#if !defined(_WIN32)
  /* Writing to wake[1] stops the accept thread's poll. On Windows, closing
   * the listening socket stops its `accept`. */
  int wake[2];
#endif
  int64_t port;
  thread_t thread;
  slug_producer *ready;
  atomic_bool closing;
  mutex_t lock;
  /* Accepted sockets not yet adopted, oldest first. */
  socket_t *queue;
  size_t queued;
  size_t capacity;
};

struct slug_tcp_connecting {
  char *host;
  char port[8];
  int64_t timeout_ms;
  slug_producer *done;
  mutex_t lock;
  /* The connect thread and the resource each hold one reference. */
  atomic_int references;
  int32_t status;
  int64_t detail;
  socket_t fd;
};

/* A connected socket: non-blocking, for the I/O thread, closed on exec, and
 * never raising SIGPIPE where the platform has a socket option for it. */
static void prepare_connected(socket_t fd) {
  set_blocking(fd, false);
  set_close_on_exec(fd);
#if defined(SO_NOSIGPIPE)
  int on = 1;
  setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, sizeof on);
#endif
}

/* The peer's address as `host:port`, or `[host]:port` for IPv6. */
static void describe_peer(socket_t fd, char *buffer, size_t size) {
  struct sockaddr_storage address;
  socklen_t length = sizeof address;
  char host[INET6_ADDRSTRLEN + 8];
  char port[8];
  buffer[0] = '\0';
  if (getpeername(fd, (struct sockaddr *)&address, &length) != 0 ||
      getnameinfo((struct sockaddr *)&address, length, host, sizeof host, port, sizeof port,
                  NI_NUMERICHOST | NI_NUMERICSERV) != 0) {
    return;
  }
  snprintf(buffer, size, address.ss_family == AF_INET6 ? "[%s]:%s" : "%s:%s", host, port);
}

static slug_tcp_connection *new_connection(socket_t fd) {
  slug_tcp_connection *connection = calloc(1, sizeof *connection);
  if (connection == NULL) {
    close_socket(fd);
    return NULL;
  }
  connection->fd = fd;
  atomic_init(&connection->references, 1);
  atomic_init(&connection->closed, false);
  atomic_init(&connection->started, false);
  mutex_init(&connection->lock);
  connection->write_timeout_ms = DEFAULT_WRITE_TIMEOUT_MS;
  describe_peer(fd, connection->peer, sizeof connection->peer);
  return connection;
}

static bool format_port(int64_t port, char *buffer, size_t size) {
  if (port < 0 || port > 65535) return false;
  snprintf(buffer, size, "%d", (int)port);
  return true;
}

/* A copy of `text`, or NULL. */
static char *copy_text(const char *text) {
  size_t length = strlen(text) + 1;
  char *copy = malloc(length);
  if (copy != NULL) memcpy(copy, text, length);
  return copy;
}

/* Sends `value`, retrying while the channel is full. Returns false when the
 * receiver has gone or `stop` is set. Not for the I/O thread, which must
 * not wait on one connection. */
static bool send_value(slug_producer *producer, int64_t value, atomic_bool *stop) {
  for (;;) {
    switch (slug_producer_send_i64(producer, value)) {
      case SLUG_PRODUCER_SENT:
        return true;
      case SLUG_PRODUCER_FULL:
        if (stop != NULL && atomic_load(stop)) return false;
        pause_briefly();
        break;
      default:
        return false;
    }
  }
}

/* --- Listening -------------------------------------------------------------- */

static bool enqueue(slug_tcp_listener *listener, socket_t fd) {
  mutex_lock(&listener->lock);
  if (atomic_load(&listener->closing)) {
    mutex_unlock(&listener->lock);
    return false;
  }
  if (listener->queued == listener->capacity) {
    size_t capacity = listener->capacity == 0 ? 16 : listener->capacity * 2;
    socket_t *queue = realloc(listener->queue, capacity * sizeof *queue);
    if (queue == NULL) {
      mutex_unlock(&listener->lock);
      return false;
    }
    listener->queue = queue;
    listener->capacity = capacity;
  }
  listener->queue[listener->queued++] = fd;
  mutex_unlock(&listener->lock);
  return true;
}

/* Waits for the next connection, or returns NO_SOCKET with `error` set: 0
 * when the listener is closing, or a fatal error. */
static socket_t next_connection(slug_tcp_listener *listener, int *error) {
#if defined(_WIN32)
  /* A blocking accept, which closing the socket ends. */
  for (;;) {
    socket_t fd = accept(listener->fd, NULL, NULL);
    if (fd != NO_SOCKET) return fd;
    *error = socket_error();
    if (atomic_load(&listener->closing)) {
      *error = 0;
      return NO_SOCKET;
    }
    if (*error == WSAEINTR || *error == WSAECONNRESET || *error == WSAEWOULDBLOCK) continue;
    if (*error == WSAEMFILE || *error == WSAENOBUFS) {
      /* Out of sockets or memory: wait for some to be released. */
      pause_briefly();
      continue;
    }
    return NO_SOCKET;
  }
#else
  struct pollfd fds[2] = {{listener->fd, POLLIN, 0}, {listener->wake[0], POLLIN, 0}};
  while (!atomic_load(&listener->closing)) {
    if (poll(fds, 2, -1) < 0) {
      if (errno == EINTR) continue;
      *error = errno;
      return NO_SOCKET;
    }
    if (fds[1].revents != 0) break;
    if (fds[0].revents == 0) continue;
    int fd = accept(listener->fd, NULL, NULL);
    if (fd >= 0) return fd;
    if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR || errno == ECONNABORTED ||
        errno == EPROTO) {
      continue;
    }
    if (errno == EMFILE || errno == ENFILE || errno == ENOBUFS || errno == ENOMEM) {
      /* Out of descriptors or memory: wait for some to be released. */
      pause_briefly();
      continue;
    }
    *error = errno;
    return NO_SOCKET;
  }
  *error = 0;
  return NO_SOCKET;
#endif
}

THREAD_FUNCTION(accept_loop) {
  slug_tcp_listener *listener = argument;
  while (!atomic_load(&listener->closing)) {
    int error = 0;
    socket_t fd = next_connection(listener, &error);
    if (fd == NO_SOCKET) {
      if (error != 0) send_value(listener->ready, -(int64_t)error, &listener->closing);
      break;
    }
    prepare_connected(fd);
    if (!enqueue(listener, fd)) {
      close_socket(fd);
      break;
    }
    if (!send_value(listener->ready, 0, &listener->closing)) break;
  }
  slug_producer_close(listener->ready);
  slug_producer_release(listener->ready);
  listener->ready = NULL;
  THREAD_RETURN;
}

/* Resolves `host` and `port` into `addresses`, returning a status. */
static int32_t resolve(const char *host, const char *port, bool passive,
                       struct addrinfo **addresses, int64_t *detail) {
  struct addrinfo hints;
  memset(&hints, 0, sizeof hints);
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_flags = AI_NUMERICSERV | (passive ? AI_PASSIVE : 0);
  int result = getaddrinfo(host[0] == '\0' ? NULL : host, port, &hints, addresses);
  if (result != 0) {
    *detail = result;
    return SLUG_TCP_ADDRESS;
  }
  return SLUG_TCP_OK;
}

/* Lets a listener take its port again at once after a restart. Windows'
 * SO_REUSEADDR would let another socket take a port in use instead, so
 * there the port is kept exclusive. */
static void claim_port(socket_t fd) {
  int on = 1;
#if defined(_WIN32)
  setsockopt(fd, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, (const char *)&on, sizeof on);
#else
  setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &on, sizeof on);
#endif
}

int32_t slug_tcp_listen(const char *host, int64_t port, int64_t backlog, slug_producer *ready,
                        slug_tcp_listener **listener, int64_t *detail) {
  *listener = NULL;
  *detail = 0;
  char service[8];
  if (!format_port(port, service, sizeof service)) {
    slug_producer_release(ready);
    *detail = EINVAL;
    return SLUG_TCP_ERROR;
  }
  int error = ensure_network();
  if (error != 0) {
    slug_producer_release(ready);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
  struct addrinfo *addresses;
  int32_t status = resolve(host, service, true, &addresses, detail);
  if (status != SLUG_TCP_OK) {
    slug_producer_release(ready);
    return status;
  }
  socket_t fd = NO_SOCKET;
  for (struct addrinfo *address = addresses; address != NULL; address = address->ai_next) {
    fd = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
    if (fd == NO_SOCKET) {
      error = socket_error();
      continue;
    }
    claim_port(fd);
    if (bind(fd, address->ai_addr, (socklen_t)address->ai_addrlen) == 0 &&
        listen(fd, backlog > 0 && backlog < 65536 ? (int)backlog : 128) == 0) {
      break;
    }
    error = socket_error();
    close_socket(fd);
    fd = NO_SOCKET;
  }
  freeaddrinfo(addresses);
  if (fd == NO_SOCKET) {
    slug_producer_release(ready);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
  set_close_on_exec(fd);

  slug_tcp_listener *created = calloc(1, sizeof *created);
#if defined(_WIN32)
  if (created == NULL) {
    error = ENOMEM;
#else
  /* The accept thread polls, so the socket never blocks it. */
  set_blocking(fd, false);
  if (created == NULL || pipe(created->wake) != 0) {
    error = created == NULL ? ENOMEM : errno;
#endif
    free(created);
    close_socket(fd);
    slug_producer_release(ready);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
#if !defined(_WIN32)
  set_close_on_exec(created->wake[0]);
  set_close_on_exec(created->wake[1]);
#endif
  created->fd = fd;
  created->ready = ready;
  atomic_init(&created->closing, false);
  mutex_init(&created->lock);
  struct sockaddr_storage bound;
  socklen_t length = sizeof bound;
  if (getsockname(fd, (struct sockaddr *)&bound, &length) == 0) {
    created->port = bound.ss_family == AF_INET6
                        ? ntohs(((struct sockaddr_in6 *)&bound)->sin6_port)
                        : ntohs(((struct sockaddr_in *)&bound)->sin_port);
  }
  if ((error = thread_start(&created->thread, accept_loop, created)) != 0) {
#if !defined(_WIN32)
    close(created->wake[0]);
    close(created->wake[1]);
#endif
    mutex_destroy(&created->lock);
    free(created);
    close_socket(fd);
    slug_producer_release(ready);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
  *listener = created;
  return SLUG_TCP_OK;
}

int64_t slug_tcp_local_port(slug_tcp_listener *listener) { return listener->port; }

int32_t slug_tcp_adopt(slug_tcp_listener *listener, slug_tcp_connection **connection) {
  *connection = NULL;
  mutex_lock(&listener->lock);
  if (listener->queued == 0) {
    mutex_unlock(&listener->lock);
    return SLUG_TCP_CLOSED;
  }
  socket_t fd = listener->queue[0];
  listener->queued--;
  memmove(listener->queue, listener->queue + 1, listener->queued * sizeof *listener->queue);
  mutex_unlock(&listener->lock);
  *connection = new_connection(fd);
  return *connection == NULL ? SLUG_TCP_CLOSED : SLUG_TCP_OK;
}

void slug_tcp_listener_release(slug_tcp_listener *listener) {
  atomic_store(&listener->closing, true);
#if defined(_WIN32)
  /* Ends the accept thread's `accept`. */
  close_socket(listener->fd);
  thread_join(listener->thread);
#else
  char wake = 0;
  while (write(listener->wake[1], &wake, 1) < 0 && errno == EINTR) {
  }
  thread_join(listener->thread);
  close(listener->fd);
  close(listener->wake[0]);
  close(listener->wake[1]);
#endif
  for (size_t i = 0; i < listener->queued; i++) close_socket(listener->queue[i]);
  free(listener->queue);
  mutex_destroy(&listener->lock);
  free(listener);
}

/* --- Connecting ------------------------------------------------------------- */

static void drop_connecting(slug_tcp_connecting *connecting) {
  if (atomic_fetch_sub(&connecting->references, 1) != 1) return;
  if (connecting->fd != NO_SOCKET) close_socket(connecting->fd);
  mutex_destroy(&connecting->lock);
  free(connecting->host);
  free(connecting);
}

/* Waits until a non-blocking connect of `fd` is done or `deadline` passes:
 * returns false at the deadline. */
static bool wait_connected(socket_t fd, int64_t deadline) {
  for (;;) {
    int64_t remaining = deadline - now_ms();
    if (remaining <= 0) return false;
#if defined(_WIN32)
    /* `select`, as WSAPoll reports no failed connect on older Windows: a
     * failure is an exception. */
    fd_set writable;
    fd_set failed;
    struct timeval timeout;
    FD_ZERO(&writable);
    FD_ZERO(&failed);
    FD_SET(fd, &writable);
    FD_SET(fd, &failed);
    timeout.tv_sec = (long)(remaining / 1000);
    timeout.tv_usec = (long)(remaining % 1000) * 1000;
    int selected = select(0, NULL, &writable, &failed, &timeout);
    if (selected <= 0) continue;
#else
    struct pollfd ready = {fd, POLLOUT, 0};
    int polled = poll(&ready, 1, remaining > 2147483647 ? 2147483647 : (int)remaining);
    if (polled <= 0) continue;
#endif
    return true;
  }
}

/* Connects to one address within `deadline`, returning the socket or
 * NO_SOCKET with `status` and `error` set. */
static socket_t connect_one(const struct addrinfo *address, int64_t deadline, int32_t *status,
                            int *error) {
  socket_t fd = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
  if (fd == NO_SOCKET) {
    *status = SLUG_TCP_ERROR;
    *error = socket_error();
    return NO_SOCKET;
  }
  set_close_on_exec(fd);
  set_blocking(fd, false);
  if (connect(fd, address->ai_addr, (socklen_t)address->ai_addrlen) != 0) {
    int failure = socket_error();
    if (failure != NET_EINPROGRESS) {
      *error = failure;
      *status = failure == NET_ECONNREFUSED ? SLUG_TCP_REFUSED : SLUG_TCP_ERROR;
      close_socket(fd);
      return NO_SOCKET;
    }
    if (!wait_connected(fd, deadline)) {
      *status = SLUG_TCP_TIMEOUT;
      *error = NET_ETIMEDOUT;
      close_socket(fd);
      return NO_SOCKET;
    }
    failure = 0;
    socklen_t length = sizeof failure;
    getsockopt(fd, SOL_SOCKET, SO_ERROR, (void *)&failure, &length);
    if (failure != 0) {
      *error = failure;
      *status = failure == NET_ECONNREFUSED ? SLUG_TCP_REFUSED : SLUG_TCP_ERROR;
      close_socket(fd);
      return NO_SOCKET;
    }
  }
  prepare_connected(fd);
  return fd;
}

THREAD_FUNCTION(connect_thread) {
  slug_tcp_connecting *connecting = argument;
  int64_t deadline = now_ms() + connecting->timeout_ms;
  struct addrinfo *addresses;
  int64_t detail = 0;
  int32_t status = resolve(connecting->host, connecting->port, false, &addresses, &detail);
  socket_t fd = NO_SOCKET;
  if (status == SLUG_TCP_OK) {
    int error = 0;
    status = SLUG_TCP_ERROR;
    for (struct addrinfo *address = addresses; address != NULL && fd == NO_SOCKET;
         address = address->ai_next) {
      fd = connect_one(address, deadline, &status, &error);
      if (status == SLUG_TCP_TIMEOUT) break;
    }
    freeaddrinfo(addresses);
    if (fd != NO_SOCKET) {
      status = SLUG_TCP_OK;
    } else {
      detail = error;
    }
  }
  mutex_lock(&connecting->lock);
  connecting->status = status;
  connecting->detail = detail;
  connecting->fd = fd;
  mutex_unlock(&connecting->lock);
  send_value(connecting->done, status, NULL);
  slug_producer_close(connecting->done);
  slug_producer_release(connecting->done);
  drop_connecting(connecting);
  THREAD_RETURN;
}

int32_t slug_tcp_connect(const char *host, int64_t port, int64_t timeout_ms, slug_producer *done,
                         slug_tcp_connecting **connecting, int64_t *detail) {
  *connecting = NULL;
  *detail = 0;
  int error = ensure_network();
  if (error != 0) {
    slug_producer_release(done);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
  slug_tcp_connecting *created = calloc(1, sizeof *created);
  if (created == NULL || (created->host = copy_text(host)) == NULL) {
    free(created);
    slug_producer_release(done);
    *detail = ENOMEM;
    return SLUG_TCP_ERROR;
  }
  if (!format_port(port, created->port, sizeof created->port)) {
    free(created->host);
    free(created);
    slug_producer_release(done);
    *detail = EINVAL;
    return SLUG_TCP_ERROR;
  }
  created->timeout_ms = timeout_ms < 0 ? 0 : timeout_ms;
  created->done = done;
  created->status = SLUG_TCP_CLOSED;
  created->fd = NO_SOCKET;
  atomic_init(&created->references, 2);
  mutex_init(&created->lock);
  thread_t thread;
  error = thread_start(&thread, connect_thread, created);
  if (error != 0) {
    mutex_destroy(&created->lock);
    free(created->host);
    free(created);
    slug_producer_release(done);
    *detail = error;
    return SLUG_TCP_ERROR;
  }
  thread_detach(thread);
  *connecting = created;
  return SLUG_TCP_OK;
}

int32_t slug_tcp_finish_connect(slug_tcp_connecting *connecting,
                                slug_tcp_connection **connection, int64_t *detail) {
  *connection = NULL;
  mutex_lock(&connecting->lock);
  int32_t status = connecting->status;
  *detail = connecting->detail;
  socket_t fd = connecting->fd;
  if (status == SLUG_TCP_OK) {
    connecting->fd = NO_SOCKET;
    /* Taken once: a second call finds no connection. */
    connecting->status = SLUG_TCP_CLOSED;
  }
  mutex_unlock(&connecting->lock);
  if (status == SLUG_TCP_OK && (*connection = new_connection(fd)) == NULL) {
    *detail = ENOMEM;
    return SLUG_TCP_ERROR;
  }
  return status;
}

void slug_tcp_connecting_release(slug_tcp_connecting *connecting) { drop_connecting(connecting); }

/* --- Connections ------------------------------------------------------------ */

static void free_pending(struct pending *pending) {
  while (pending != NULL) {
    struct pending *next = pending->next;
    free(pending);
    pending = next;
  }
}

static void drop_connection(slug_tcp_connection *connection) {
  if (atomic_fetch_sub(&connection->references, 1) != 1) return;
  free_pending(connection->first);
  free(connection->held);
  mutex_destroy(&connection->lock);
  close_socket(connection->fd);
  free(connection);
}

/* The I/O thread. */
static struct {
#if defined(_WIN32)
  INIT_ONCE once;
  /* A UDP socket connected to itself: a byte sent on it wakes the thread to
   * look at `changed`. */
  socket_t wake;
  /* The I/O thread's own: the connections whose sockets it polls. */
  slug_tcp_connection **watched;
  size_t watched_count;
  size_t watched_capacity;
#else
  pthread_once_t once;
  int poller;
  /* Writing to wake[1] wakes the thread to look at `changed`. */
  int wake[2];
#endif
  /* The error starting it failed with, or 0. */
  int error;
  thread_t thread;
  mutex_t lock;
  /* Connections to look at, each once, and whether a wake is on its way. */
  slug_tcp_connection *changed;
  bool woken;
  /* The I/O thread's own: connections holding a chunk, and waiting to send. */
  slug_tcp_connection *held;
  slug_tcp_connection *blocked;
  slug_tcp_connection *retired;
  uint8_t buffer[READ_BUFFER];
#if defined(_WIN32)
} io = {.once = INIT_ONCE_STATIC_INIT, .lock = MUTEX_INITIALIZER};
#else
} io = {.once = PTHREAD_ONCE_INIT, .lock = MUTEX_INITIALIZER};
#endif

/* Hands `connection` to the I/O thread, which looks at what it must do: start
 * it, send what is queued, shut it down, or finish it once closed. */
static void request(slug_tcp_connection *connection) {
  mutex_lock(&io.lock);
  if (!connection->changed) {
    connection->changed = true;
    atomic_fetch_add(&connection->references, 1);
    connection->next_changed = io.changed;
    io.changed = connection;
  }
  bool wake = !io.woken;
  io.woken = true;
  mutex_unlock(&io.lock);
  if (wake) {
    char byte = 0;
#if defined(_WIN32)
    send(io.wake, &byte, 1, 0);
#else
    while (write(io.wake[1], &byte, 1) < 0 && errno == EINTR) {
    }
#endif
  }
}

#if defined(_WIN32)

/* Adds `connection` to the sockets the I/O thread polls; false when memory
 * ran out, which leaves it unwatched until it is asked again. */
static bool watch(slug_tcp_connection *connection) {
  if (io.watched_count == io.watched_capacity) {
    size_t capacity = io.watched_capacity == 0 ? 64 : io.watched_capacity * 2;
    slug_tcp_connection **grown = realloc(io.watched, capacity * sizeof *grown);
    if (grown == NULL) return false;
    io.watched = grown;
    io.watched_capacity = capacity;
  }
  connection->watch_index = io.watched_count;
  io.watched[io.watched_count++] = connection;
  return true;
}

static void unwatch(slug_tcp_connection *connection) {
  slug_tcp_connection *last = io.watched[--io.watched_count];
  io.watched[connection->watch_index] = last;
  last->watch_index = connection->watch_index;
}

#endif

/* Tells the kernel which of reading and writing `connection` waits for. */
static void set_interest(slug_tcp_connection *connection, bool read, bool write) {
  if (connection->registered && connection->want_read == read && connection->want_write == write) {
    return;
  }
#if defined(_WIN32)
  /* As with epoll below, a socket waiting for nothing is not polled, since
   * a hang-up is reported even then. */
  if (read || write) {
    if (!connection->registered && !watch(connection)) return;
    connection->registered = true;
  } else if (connection->registered) {
    unwatch(connection);
    connection->registered = false;
  }
#elif defined(SLUG_TCP_KQUEUE)
  struct kevent changes[2];
  int count = 0;
  if (!connection->registered) {
    EV_SET(&changes[count++], connection->fd, EVFILT_READ, EV_ADD | (read ? EV_ENABLE : EV_DISABLE),
           0, 0, connection);
    EV_SET(&changes[count++], connection->fd, EVFILT_WRITE,
           EV_ADD | (write ? EV_ENABLE : EV_DISABLE), 0, 0, connection);
  } else {
    if (connection->want_read != read) {
      EV_SET(&changes[count++], connection->fd, EVFILT_READ, read ? EV_ENABLE : EV_DISABLE, 0, 0,
             connection);
    }
    if (connection->want_write != write) {
      EV_SET(&changes[count++], connection->fd, EVFILT_WRITE, write ? EV_ENABLE : EV_DISABLE, 0, 0,
             connection);
    }
  }
  kevent(io.poller, changes, count, NULL, 0, NULL);
  connection->registered = true;
#else
  /* epoll reports a hang-up even to a socket it waits on for nothing, so a
   * connection that waits for nothing leaves it. */
  struct epoll_event event = {0};
  event.events = (read ? EPOLLIN : 0) | (write ? EPOLLOUT : 0);
  event.data.ptr = connection;
  if (!read && !write) {
    if (connection->registered) epoll_ctl(io.poller, EPOLL_CTL_DEL, connection->fd, NULL);
    connection->registered = false;
  } else {
    epoll_ctl(io.poller, connection->registered ? EPOLL_CTL_MOD : EPOLL_CTL_ADD, connection->fd,
              &event);
    connection->registered = true;
  }
#endif
  connection->want_read = read;
  connection->want_write = write;
}

static void deregister(slug_tcp_connection *connection) {
  if (!connection->registered) return;
#if defined(_WIN32)
  unwatch(connection);
#elif defined(SLUG_TCP_KQUEUE)
  struct kevent changes[2];
  EV_SET(&changes[0], connection->fd, EVFILT_READ, EV_DELETE, 0, 0, NULL);
  EV_SET(&changes[1], connection->fd, EVFILT_WRITE, EV_DELETE, 0, 0, NULL);
  kevent(io.poller, changes, 2, NULL, 0, NULL);
#else
  epoll_ctl(io.poller, EPOLL_CTL_DEL, connection->fd, NULL);
#endif
  connection->registered = false;
}

static void update_interest(slug_tcp_connection *connection) {
  bool read = connection->reading && !connection->holding;
  bool write = connection->on_blocked;
  set_interest(connection, read, write);
}

static void list_held(slug_tcp_connection *connection, bool on) {
  if (connection->on_held == on) return;
  if (on) {
    connection->prev_held = NULL;
    connection->next_held = io.held;
    if (io.held != NULL) io.held->prev_held = connection;
    io.held = connection;
  } else {
    if (connection->prev_held != NULL) {
      connection->prev_held->next_held = connection->next_held;
    } else {
      io.held = connection->next_held;
    }
    if (connection->next_held != NULL) connection->next_held->prev_held = connection->prev_held;
  }
  connection->on_held = on;
}

static void list_blocked(slug_tcp_connection *connection, bool on) {
  if (connection->on_blocked == on) return;
  if (on) {
    connection->prev_blocked = NULL;
    connection->next_blocked = io.blocked;
    if (io.blocked != NULL) io.blocked->prev_blocked = connection;
    io.blocked = connection;
  } else {
    if (connection->prev_blocked != NULL) {
      connection->prev_blocked->next_blocked = connection->next_blocked;
    } else {
      io.blocked = connection->next_blocked;
    }
    if (connection->next_blocked != NULL) {
      connection->next_blocked->prev_blocked = connection->prev_blocked;
    }
  }
  connection->on_blocked = on;
}

static void stop_reading(slug_tcp_connection *connection);

/* Once neither reading nor writing remains, the I/O thread lets go of the
 * connection, at the end of the round. */
static void finish_if_done(slug_tcp_connection *connection) {
  if (connection->finished) return;
#if !defined(_WIN32)
  if (connection->reading) return;
#endif
  mutex_lock(&connection->lock);
  bool writing = connection->writing;
  mutex_unlock(&connection->lock);
  if (writing) return;
#if defined(_WIN32)
  /* A shutdown ends a POSIX socket's reading by waking its read with the
   * end of the stream; Winsock may wake nothing, so a closed connection with
   * nothing left to send stops reading here. */
  if (connection->reading && atomic_load(&connection->closed)) {
    stop_reading(connection);
    return;
  }
#endif
  if (connection->reading) return;
  connection->finished = true;
  list_held(connection, false);
  list_blocked(connection, false);
  deregister(connection);
  connection->next_retired = io.retired;
  io.retired = connection;
}

static void stop_reading(slug_tcp_connection *connection) {
  if (!connection->reading) return;
  connection->reading = false;
  free(connection->held);
  connection->held = NULL;
  connection->holding = false;
  list_held(connection, false);
  slug_producer_close(connection->chunks);
  slug_producer_release(connection->chunks);
  connection->chunks = NULL;
  update_interest(connection);
  finish_if_done(connection);
}

/* With the lock held: tells a waiting `write` that the queue shrank, that
 * writing failed, or with TICK_STALLED that the peer took nothing for the
 * write timeout. A tick already in the channel will do. */
#define TICK_PROGRESS 0
#define TICK_STALLED 1
static void notify_writable(slug_tcp_connection *connection, int64_t tick) {
  if (!connection->waiting || connection->writable == NULL) return;
  connection->waiting = false;
  slug_producer_send_i64(connection->writable, tick);
}

/* With the lock held: ends writing, after which writes fail. Once closed, the
 * socket shuts down, which ends reading too and lets the peer see the end. */
static void stop_writing(slug_tcp_connection *connection) {
  connection->writing = false;
  connection->blocked = false;
  slug_producer *writable = connection->writable;
  connection->writable = NULL;
  slug_producer_close(writable);
  slug_producer_release(writable);
  if (atomic_load(&connection->closed)) shutdown(connection->fd, SHUT_RDWR);
}

/* Sends what is queued until the peer takes no more, shuts down the sending
 * side when asked once the queue is empty, and ends writing once the
 * connection is closed and everything is sent. */
static void flush(slug_tcp_connection *connection) {
  mutex_lock(&connection->lock);
  if (!connection->writing) {
    mutex_unlock(&connection->lock);
    list_blocked(connection, false);
    update_interest(connection);
    finish_if_done(connection);
    return;
  }
  bool blocked = false;
  bool progress = false;
  for (;;) {
    struct pending *pending = connection->first;
    if (pending == NULL) {
      if (connection->shutdown_requested && !connection->shut_down) {
        shutdown(connection->fd, SHUT_WR);
        connection->shut_down = true;
      }
      break;
    }
    size_t length = pending->length - pending->sent;
    if (length > WRITE_SLICE) length = WRITE_SLICE;
    int64_t sent = socket_send(connection->fd, pending->data + pending->sent, length);
    if (sent < 0) {
      int error = socket_error();
      if (error == NET_EINTR) continue;
      if (would_block(error)) {
        blocked = true;
        break;
      }
      connection->write_error = error;
      free_pending(connection->first);
      connection->first = connection->last = NULL;
      connection->queued = 0;
      notify_writable(connection, TICK_PROGRESS);
      stop_writing(connection);
      break;
    }
    progress = true;
    pending->sent += (size_t)sent;
    connection->queued -= (size_t)sent;
    if (pending->sent == pending->length) {
      connection->first = pending->next;
      if (connection->first == NULL) connection->last = NULL;
      free(pending);
    }
  }
  if (progress) notify_writable(connection, TICK_PROGRESS);
  if (connection->writing && connection->first == NULL && atomic_load(&connection->closed)) {
    stop_writing(connection);
  }
  connection->blocked = blocked && connection->writing;
  bool now_blocked = connection->blocked;
  mutex_unlock(&connection->lock);
  if (now_blocked && (progress || !connection->on_blocked)) connection->progress_ms = now_ms();
  list_blocked(connection, now_blocked);
  update_interest(connection);
  finish_if_done(connection);
}

/* Sends the chunk or error the channel had no room for; returns whether it
 * went, or reading has stopped. */
static bool send_held(slug_tcp_connection *connection) {
  int32_t result = connection->held_error != 0
                       ? slug_producer_send_i64(connection->chunks, connection->held_error)
                       : slug_producer_send_bytes(connection->chunks, connection->held,
                                                  connection->held_length);
  if (result == SLUG_PRODUCER_FULL) return false;
  bool failed = connection->held_error != 0;
  free(connection->held);
  connection->held = NULL;
  connection->holding = false;
  list_held(connection, false);
  /* A failed receive, or a receiver that has gone, ends reading. */
  if (result != SLUG_PRODUCER_SENT || failed) {
    stop_reading(connection);
  } else {
    update_interest(connection);
  }
  return true;
}

/* Holds what the chunk channel has no room for, and stops reading the socket
 * until it does: the peer's sending slows. */
static void hold(slug_tcp_connection *connection, const uint8_t *data, size_t length, int error) {
  if (atomic_load(&connection->closed)) {
    stop_reading(connection);
    return;
  }
  connection->held_error = error;
  connection->held_length = length;
  connection->held = NULL;
  if (length > 0) {
    connection->held = malloc(length);
    if (connection->held == NULL) {
      stop_reading(connection);
      return;
    }
    memcpy(connection->held, data, length);
  }
  connection->holding = true;
  list_held(connection, true);
  update_interest(connection);
}

/* Receives what the socket has, at most one chunk, and sends it on. */
static void receive(slug_tcp_connection *connection) {
  if (!connection->reading || connection->holding) return;
  int64_t received = socket_receive(connection->fd, io.buffer, READ_BUFFER);
  if (received == 0) {
    stop_reading(connection);
    return;
  }
  if (received < 0) {
    int error = socket_error();
    if (error == NET_EINTR || would_block(error)) return;
    /* A shutdown by `close` is not a failure. */
    if (atomic_load(&connection->closed)) {
      stop_reading(connection);
      return;
    }
    switch (slug_producer_send_i64(connection->chunks, error)) {
      case SLUG_PRODUCER_FULL:
        hold(connection, NULL, 0, error);
        break;
      default:
        stop_reading(connection);
    }
    return;
  }
  switch (slug_producer_send_bytes(connection->chunks, io.buffer, (size_t)received)) {
    case SLUG_PRODUCER_SENT:
      break;
    case SLUG_PRODUCER_FULL:
      hold(connection, io.buffer, (size_t)received, 0);
      break;
    default:
      stop_reading(connection);
  }
}

/* Looks at a connection another thread handed over. */
static void look_at(slug_tcp_connection *connection) {
  if (connection->finished) return;
  /* A connection just started begins waiting for what arrives. */
  update_interest(connection);
  /* Once closed, a held chunk will not be read. */
  if (connection->holding && atomic_load(&connection->closed)) stop_reading(connection);
  flush(connection);
}

/* Sends held chunks the channels now have room for. */
static void retry_held(void) {
  slug_tcp_connection *connection = io.held;
  while (connection != NULL) {
    slug_tcp_connection *next = connection->next_held;
    if (atomic_load(&connection->closed)) {
      stop_reading(connection);
    } else {
      send_held(connection);
    }
    connection = next;
  }
}

/* Writes that the peer has taken nothing of for the write timeout: a waiting
 * `write` is told, or once the connection is closed, writing ends with a
 * timeout. Returns the milliseconds until the next such timeout, or -1. */
static int64_t check_timeouts(void) {
  int64_t now = now_ms();
  int64_t next = -1;
  slug_tcp_connection *connection = io.blocked;
  while (connection != NULL) {
    slug_tcp_connection *following = connection->next_blocked;
    mutex_lock(&connection->lock);
    int64_t timeout = connection->write_timeout_ms;
    bool expired = timeout > 0 && now - connection->progress_ms >= timeout;
    bool ended = false;
    if (expired) {
      if (atomic_load(&connection->closed)) {
        connection->write_error = NET_ETIMEDOUT;
        free_pending(connection->first);
        connection->first = connection->last = NULL;
        connection->queued = 0;
        notify_writable(connection, TICK_PROGRESS);
        stop_writing(connection);
        ended = true;
      } else {
        notify_writable(connection, TICK_STALLED);
        connection->progress_ms = now;
      }
    }
    mutex_unlock(&connection->lock);
    if (ended) {
      list_blocked(connection, false);
      update_interest(connection);
      finish_if_done(connection);
    } else if (timeout > 0) {
      int64_t left = connection->progress_ms + timeout - now;
      if (left < 0) left = 0;
      if (next < 0 || left < next) next = left;
    }
    connection = following;
  }
  return next;
}

/* The end of the I/O thread's round: what other threads handed over, held
 * chunks, and the connections it finished with. */
static void finish_round(void) {
  mutex_lock(&io.lock);
  slug_tcp_connection *changed = io.changed;
  io.changed = NULL;
  io.woken = false;
  mutex_unlock(&io.lock);
  while (changed != NULL) {
    /* Once its flag is clear, another thread may hand the connection over
     * again, onto the next list, which overwrites its link: take the link
     * first. */
    mutex_lock(&io.lock);
    slug_tcp_connection *next = changed->next_changed;
    changed->changed = false;
    mutex_unlock(&io.lock);
    look_at(changed);
    /* The list's reference. A connection the I/O thread finished with
     * keeps its own reference until the retired are dropped. */
    drop_connection(changed);
    changed = next;
  }
  if (io.held != NULL) retry_held();
  while (io.retired != NULL) {
    slug_tcp_connection *connection = io.retired;
    io.retired = connection->next_retired;
    drop_connection(connection);
  }
}

/* How long the I/O thread may wait for events: until the next write timeout,
 * and briefly while a chunk waits for room. */
static int64_t wait_time(void) {
  int64_t wait_ms = check_timeouts();
  if (io.held != NULL && (wait_ms < 0 || wait_ms > 1)) wait_ms = 1;
  return wait_ms;
}

/* Handles a socket's readiness. */
static void ready(slug_tcp_connection *connection, bool readable, bool writable) {
  if (connection->finished) return;
  if (readable) receive(connection);
  if (writable && connection->on_blocked) flush(connection);
}

#if defined(_WIN32)

THREAD_FUNCTION(io_loop) {
  (void)argument;
  WSAPOLLFD *fds = NULL;
  slug_tcp_connection **owners = NULL;
  size_t capacity = 0;
  for (;;) {
    int64_t wait_ms = wait_time();
    size_t wanted = io.watched_count + 1;
    if (wanted > capacity) {
      size_t grown_capacity = wanted * 2;
      WSAPOLLFD *grown_fds = realloc(fds, grown_capacity * sizeof *grown_fds);
      if (grown_fds != NULL) fds = grown_fds;
      slug_tcp_connection **grown_owners =
          grown_fds == NULL ? NULL : realloc(owners, grown_capacity * sizeof *grown_owners);
      if (grown_owners != NULL) owners = grown_owners;
      if (grown_fds == NULL || grown_owners == NULL) {
        /* Without memory to poll them all, wait for some to be released. */
        pause_briefly();
        finish_round();
        continue;
      }
      capacity = grown_capacity;
    }
    size_t used = 0;
    fds[used].fd = io.wake;
    fds[used].events = POLLRDNORM;
    fds[used].revents = 0;
    owners[used++] = NULL;
    for (size_t i = 0; i < io.watched_count; i++) {
      slug_tcp_connection *connection = io.watched[i];
      fds[used].fd = connection->fd;
      fds[used].events =
          (SHORT)((connection->want_read ? POLLRDNORM : 0) | (connection->want_write ? POLLWRNORM : 0));
      fds[used].revents = 0;
      owners[used++] = connection;
    }
    int count = WSAPoll(fds, (ULONG)used,
                        wait_ms < 0 ? -1 : (wait_ms > INT_MAX ? INT_MAX : (INT)wait_ms));
    for (size_t i = 0; count > 0 && i < used; i++) {
      SHORT revents = fds[i].revents;
      if (revents == 0) continue;
      if (owners[i] == NULL) {
        char drain[64];
        while (recv(io.wake, drain, (int)sizeof drain, 0) > 0) {
        }
        continue;
      }
      ready(owners[i], (revents & (POLLRDNORM | POLLHUP | POLLERR)) != 0,
            (revents & (POLLWRNORM | POLLHUP | POLLERR)) != 0);
    }
    finish_round();
  }
  THREAD_RETURN;
}

/* Makes the wake socket: UDP on the loopback, connected to itself. */
static int make_wake(void) {
  struct sockaddr_in address;
  int length = sizeof address;
  io.wake = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
  if (io.wake == NO_SOCKET) return socket_error();
  memset(&address, 0, sizeof address);
  address.sin_family = AF_INET;
  address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  address.sin_port = 0;
  if (bind(io.wake, (struct sockaddr *)&address, sizeof address) != 0 ||
      getsockname(io.wake, (struct sockaddr *)&address, &length) != 0 ||
      connect(io.wake, (struct sockaddr *)&address, sizeof address) != 0) {
    return socket_error();
  }
  set_close_on_exec(io.wake);
  set_blocking(io.wake, false);
  return 0;
}

static void start_io(void) {
  io.error = ensure_network();
  if (io.error != 0) return;
  io.error = make_wake();
  if (io.error != 0) return;
  io.error = thread_start(&io.thread, io_loop, NULL);
  if (io.error != 0) return;
  thread_detach(io.thread);
}

static BOOL CALLBACK start_io_once(PINIT_ONCE once, PVOID parameter, PVOID *context) {
  (void)once;
  (void)parameter;
  (void)context;
  start_io();
  return TRUE;
}

static void ensure_io(void) { InitOnceExecuteOnce(&io.once, start_io_once, NULL, NULL); }

#else

THREAD_FUNCTION(io_loop) {
  (void)argument;
#if defined(SLUG_TCP_KQUEUE)
  struct kevent events[EVENTS];
#else
  struct epoll_event events[EVENTS];
#endif
  for (;;) {
    int64_t wait_ms = wait_time();
#if defined(SLUG_TCP_KQUEUE)
    struct timespec timeout = {(time_t)(wait_ms / 1000), (long)(wait_ms % 1000) * 1000000};
    int count = kevent(io.poller, NULL, 0, events, EVENTS, wait_ms < 0 ? NULL : &timeout);
#else
    int count = epoll_wait(io.poller, events, EVENTS, wait_ms < 0 ? -1 : (int)wait_ms);
#endif
    if (count < 0) count = 0;
    for (int i = 0; i < count; i++) {
#if defined(SLUG_TCP_KQUEUE)
      slug_tcp_connection *connection = events[i].udata;
      bool readable = events[i].filter == EVFILT_READ;
      bool writable = events[i].filter == EVFILT_WRITE;
#else
      slug_tcp_connection *connection = events[i].data.ptr;
      bool readable = (events[i].events & (EPOLLIN | EPOLLHUP | EPOLLERR)) != 0;
      bool writable = (events[i].events & (EPOLLOUT | EPOLLHUP | EPOLLERR)) != 0;
#endif
      if (connection == NULL) {
        char drain[64];
        while (read(io.wake[0], drain, sizeof drain) > 0) {
        }
        continue;
      }
      ready(connection, readable, writable);
    }
    finish_round();
  }
  THREAD_RETURN;
}

static void start_io(void) {
#if defined(SLUG_TCP_KQUEUE)
  io.poller = kqueue();
#else
  io.poller = epoll_create1(EPOLL_CLOEXEC);
#endif
  if (io.poller < 0) {
    io.error = errno;
    return;
  }
  set_close_on_exec(io.poller);
  if (pipe(io.wake) != 0) {
    io.error = errno;
    return;
  }
  set_close_on_exec(io.wake[0]);
  set_close_on_exec(io.wake[1]);
  set_blocking(io.wake[0], false);
  set_blocking(io.wake[1], false);
#if defined(SLUG_TCP_KQUEUE)
  struct kevent change;
  EV_SET(&change, io.wake[0], EVFILT_READ, EV_ADD, 0, 0, NULL);
  if (kevent(io.poller, &change, 1, NULL, 0, NULL) != 0) {
    io.error = errno;
    return;
  }
#else
  struct epoll_event event = {0};
  event.events = EPOLLIN;
  event.data.ptr = NULL;
  if (epoll_ctl(io.poller, EPOLL_CTL_ADD, io.wake[0], &event) != 0) {
    io.error = errno;
    return;
  }
#endif
  int error = thread_start(&io.thread, io_loop, NULL);
  if (error != 0) {
    io.error = error;
    return;
  }
  thread_detach(io.thread);
}

static void ensure_io(void) { pthread_once(&io.once, start_io); }

#endif

void slug_tcp_connection_release(slug_tcp_connection *connection) {
  atomic_store(&connection->closed, true);
  mutex_lock(&connection->lock);
  bool writing = connection->writing;
  mutex_unlock(&connection->lock);
  /* Without writing to finish, shut down now, which ends reading. Otherwise
   * the I/O thread shuts down once it has sent what is queued. */
  if (!writing) shutdown(connection->fd, SHUT_RDWR);
  if (atomic_load(&connection->started)) request(connection);
  drop_connection(connection);
}

int32_t slug_tcp_start(slug_tcp_connection *connection, slug_producer *chunks,
                       slug_producer *writable, int64_t *detail) {
  *detail = 0;
  if (atomic_load(&connection->started)) {
    slug_producer_release(chunks);
    slug_producer_release(writable);
    return SLUG_TCP_OK;
  }
  ensure_io();
  if (io.error != 0) {
    slug_producer_release(chunks);
    slug_producer_release(writable);
    *detail = io.error;
    return SLUG_TCP_ERROR;
  }
  if (atomic_exchange(&connection->started, true)) {
    slug_producer_release(chunks);
    slug_producer_release(writable);
    return SLUG_TCP_OK;
  }
  connection->chunks = chunks;
  connection->reading = true;
  mutex_lock(&connection->lock);
  connection->writable = writable;
  connection->writing = true;
  mutex_unlock(&connection->lock);
  /* The I/O thread's reference, until it has finished with the connection. */
  atomic_fetch_add(&connection->references, 1);
  request(connection);
  return SLUG_TCP_OK;
}

/* With the lock held: whether the connection can still take writes. */
static int32_t writable_status(slug_tcp_connection *connection, int64_t *detail) {
  if (connection->write_error != 0) {
    *detail = connection->write_error;
    return *detail == NET_ETIMEDOUT ? SLUG_TCP_TIMEOUT : SLUG_TCP_ERROR;
  }
  if (connection->shutdown_requested) {
    *detail = NET_EPIPE;
    return SLUG_TCP_ERROR;
  }
  return connection->writing ? SLUG_TCP_OK : SLUG_TCP_CLOSED;
}

/* Queues a copy of `data` for the I/O thread, writing the bytes now queued. */
static int32_t queue_write(slug_tcp_connection *connection, const uint8_t *data, size_t length,
                           int64_t *queued, int64_t *detail) {
  *detail = 0;
  mutex_lock(&connection->lock);
  *queued = (int64_t)connection->queued;
  int32_t status = writable_status(connection, detail);
  if (status != SLUG_TCP_OK) {
    mutex_unlock(&connection->lock);
    return status;
  }
  bool hand_over = false;
  if (length > 0) {
    struct pending *pending = malloc(sizeof *pending + length);
    if (pending == NULL) {
      mutex_unlock(&connection->lock);
      *detail = ENOMEM;
      return SLUG_TCP_ERROR;
    }
    pending->next = NULL;
    pending->length = length;
    pending->sent = 0;
    memcpy(pending->data, data, length);
    /* Bytes already queued are on their way: the I/O thread sends the whole
     * queue when it looks, or waits for the peer to take more. */
    hand_over = connection->first == NULL;
    if (connection->last == NULL) {
      connection->first = pending;
    } else {
      connection->last->next = pending;
    }
    connection->last = pending;
    connection->queued += length;
    *queued = (int64_t)connection->queued;
  }
  mutex_unlock(&connection->lock);
  if (hand_over) request(connection);
  return SLUG_TCP_OK;
}

int32_t slug_tcp_write(slug_tcp_connection *connection, const uint8_t *data, size_t length,
                       int64_t *queued, int64_t *detail) {
  return queue_write(connection, data, length, queued, detail);
}

int32_t slug_tcp_write_text(slug_tcp_connection *connection, const char *text, size_t length,
                            int64_t *queued, int64_t *detail) {
  return queue_write(connection, (const uint8_t *)text, length, queued, detail);
}

int32_t slug_tcp_wait_writable(slug_tcp_connection *connection, int64_t *queued,
                               int64_t *detail) {
  *detail = 0;
  mutex_lock(&connection->lock);
  *queued = (int64_t)connection->queued;
  int32_t status = writable_status(connection, detail);
  if (status == SLUG_TCP_OK) connection->waiting = true;
  mutex_unlock(&connection->lock);
  return status;
}

void slug_tcp_set_write_timeout(slug_tcp_connection *connection, int64_t milliseconds) {
  mutex_lock(&connection->lock);
  connection->write_timeout_ms = milliseconds < 0 ? 0 : milliseconds;
  mutex_unlock(&connection->lock);
}

int32_t slug_tcp_shutdown_write(slug_tcp_connection *connection, int64_t *detail) {
  *detail = 0;
  mutex_lock(&connection->lock);
  int32_t status = writable_status(connection, detail);
  bool hand_over = false;
  if (status == SLUG_TCP_OK) {
    /* The I/O thread shuts down once it has sent what is queued. */
    connection->shutdown_requested = true;
    hand_over = true;
  } else if (connection->shutdown_requested) {
    status = SLUG_TCP_OK;
    *detail = 0;
  }
  mutex_unlock(&connection->lock);
  if (hand_over) request(connection);
  return status;
}

const char *slug_tcp_peer(slug_tcp_connection *connection) { return connection->peer; }

#if defined(_WIN32)

/* The system's description of a Winsock or Windows error, as UTF-8, or the
 * C library's for its own numbers; the system's is in the user's language,
 * which a code page may not hold, so it is taken as UTF-16. */
static const char *system_message(int64_t detail) {
  static _Thread_local char message[512];
  wchar_t wide[256];
  DWORD length;
  if (detail > 0 && detail < 10000) {
    /* MSVC's C library deprecates `strerror`, which shares one buffer. */
    if (strerror_s(message, sizeof message, (int)detail) != 0) {
      snprintf(message, sizeof message, "error %lld", (long long)detail);
    }
    return message;
  }
  length = FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, NULL,
                          (DWORD)detail, 0, wide, sizeof wide / sizeof wide[0], NULL);
  /* Without the trailing ".\r\n" the system adds. */
  while (length > 0 &&
         (wide[length - 1] == L'\r' || wide[length - 1] == L'\n' || wide[length - 1] == L'.')) {
    length--;
  }
  int written = length == 0 ? 0
                            : WideCharToMultiByte(CP_UTF8, 0, wide, (int)length, message,
                                                  (int)sizeof message - 1, NULL, NULL);
  if (written <= 0) {
    snprintf(message, sizeof message, "system error %lld", (long long)detail);
    return message;
  }
  message[written] = '\0';
  return message;
}

#endif

const char *slug_tcp_message(int32_t status, int64_t detail) {
  switch (status) {
    case SLUG_TCP_OK:
      return "success";
    case SLUG_TCP_ADDRESS:
#if defined(_WIN32)
      /* getaddrinfo's errors are Winsock errors there. */
      return system_message(detail);
#else
      return gai_strerror((int)detail);
#endif
    case SLUG_TCP_TIMEOUT:
      return "the operation timed out";
    case SLUG_TCP_CLOSED:
      return "the connection is closed";
    default:
#if defined(_WIN32)
      return system_message(detail);
#else
      return strerror((int)detail);
#endif
  }
}

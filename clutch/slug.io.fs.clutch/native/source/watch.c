#if defined(__APPLE__)
#define _DARWIN_C_SOURCE
#endif

#include "slug_io_fs.h"
#include <stdlib.h>

#if defined(__APPLE__)
#include <CoreServices/CoreServices.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define WATCH_PATHS 1024u
#define WATCH_BYTES (1024u * 1024u)

struct slug_fs_watch {
  /* The resource and the worker each own a reference. The dispatch queue is
   * quiesced before the worker lets its reference go. */
  atomic_int references;
  pthread_mutex_t mutex;
  pthread_cond_t condition;
  char *input;
  char *root;
  bool recursive;
  bool started;
  bool stopped;
  int error;
  slug_producer *ready;
  char *excluded[WATCH_PATHS];
  size_t excluded_count;
  size_t excluded_bytes;
  char *paths[WATCH_PATHS];
  size_t count;
  size_t bytes;
  bool rescan;
};

static void clear_paths(slug_fs_watch *watch) {
  for (size_t i = 0; i < watch->count; i++) free(watch->paths[i]);
  watch->count = 0;
  watch->bytes = 0;
}

static void unref(slug_fs_watch *watch) {
  if (atomic_fetch_sub(&watch->references, 1) != 1) return;
  slug_producer_close(watch->ready);
  slug_producer_release(watch->ready);
  clear_paths(watch);
  for (size_t i = 0; i < watch->excluded_count; i++) free(watch->excluded[i]);
  free(watch->input);
  free(watch->root);
  pthread_cond_destroy(&watch->condition);
  pthread_mutex_destroy(&watch->mutex);
  free(watch);
}

static const char *error_code(int error) {
  switch (error) {
    case ENOENT: return "not_found";
    case ENOTDIR: return "not_directory";
    case EACCES: case EPERM: return "permission";
    case ELOOP: return "link_loop";
    case EINVAL: return "invalid";
    case ESTALE: return "watch_lost";
    case EMFILE: case ENFILE: return "too_many_open";
    default: return "io";
  }
}

static void reply_error(slug_producer *reply, int error) {
  if (error == ENOMEM) {
    slug_producer_item item = slug_producer_item_i64(SLUG_FS_ALLOC);
    slug_producer_send_list(reply, &item, 1);
  } else {
    const char *code = error_code(error);
    /* strerror need not be thread-safe; the Slug wrapper supplies context. */
    const char *message = error == ESTALE ? "the watched directory was removed or replaced" :
                          error == ELOOP ? "the watched directory is a symbolic link" :
                          error == ENOENT ? "no such file or directory" :
                          error == ENOTDIR ? "not a directory" :
                          error == EACCES || error == EPERM ? "permission denied" :
                          "cannot monitor the directory";
    slug_producer_item items[] = {
      slug_producer_item_i64(SLUG_FS_SYSTEM), slug_producer_item_i64(error),
      slug_producer_item_text(code, strlen(code)),
      slug_producer_item_text(message, strlen(message)),
    };
    slug_producer_send_list(reply, items, 4);
  }
}

static void finish_reply(slug_producer *reply) {
  slug_producer_close(reply);
  slug_producer_release(reply);
}

/* Called with the mutex held. Overflow state never depends on a free slot in
 * the readiness channel; FULL means that a wakeup is already pending. */
static void notify(slug_fs_watch *watch) {
  slug_producer_send_bool(watch->ready, true);
}

static void invalidate_paths(slug_fs_watch *watch) {
  clear_paths(watch);
  watch->rescan = true;
  notify(watch);
}

static bool excluded(slug_fs_watch *watch, const char *path) {
  for (size_t i = 0; i < watch->excluded_count; i++) {
    size_t length = strlen(watch->excluded[i]);
    if (strncmp(path, watch->excluded[i], length) == 0 &&
        (path[length] == 0 || path[length] == '/')) return true;
  }
  return false;
}

static void record_path(slug_fs_watch *watch, const char *path) {
  if (excluded(watch, path)) return;
  /* FSEvents is recursive. A nonrecursive subscription only reports direct
   * children, and ignores changes deeper inside those children. */
  if (!watch->recursive && strchr(path, '/') != NULL) return;
  if (watch->rescan) { notify(watch); return; }
  for (size_t i = 0; i < watch->count; i++) {
    if (strcmp(path, watch->paths[i]) == 0) { notify(watch); return; }
  }
  size_t length = strlen(path) + 1;
  if (watch->count == WATCH_PATHS || length > WATCH_BYTES - watch->bytes) {
    invalidate_paths(watch);
    return;
  }
  char *copy = strdup(path);
  if (copy == NULL) { invalidate_paths(watch); return; }
  watch->paths[watch->count++] = copy;
  watch->bytes += length;
  notify(watch);
}

static void events(ConstFSEventStreamRef stream, void *context, size_t count,
                   void *event_paths, const FSEventStreamEventFlags flags[],
                   const FSEventStreamEventId ids[]) {
  (void)stream;
  (void)ids;
  slug_fs_watch *watch = context;
  char **paths = event_paths;
  pthread_mutex_lock(&watch->mutex);
  if (watch->stopped || watch->error != 0) goto done;
  for (size_t i = 0; i < count; i++) {
    if (flags[i] & (kFSEventStreamEventFlagRootChanged | kFSEventStreamEventFlagUnmount)) {
      watch->error = ESTALE;
      notify(watch);
      pthread_cond_signal(&watch->condition);
      break;
    }
    if (flags[i] & (kFSEventStreamEventFlagMustScanSubDirs |
                    kFSEventStreamEventFlagUserDropped |
                    kFSEventStreamEventFlagKernelDropped |
                    kFSEventStreamEventFlagEventIdsWrapped)) {
      invalidate_paths(watch);
      continue;
    }
    size_t length = strlen(watch->root);
    if (strcmp(paths[i], watch->root) == 0) {
      record_path(watch, "");
    } else if (strcmp(watch->root, "/") == 0 && paths[i][0] == '/') {
      record_path(watch, paths[i] + 1);
    } else if (strncmp(paths[i], watch->root, length) == 0 && paths[i][length] == '/') {
      record_path(watch, paths[i] + length + 1);
    } else {
      /* Never expose an absolute/outside path as a relative invalidation. */
      invalidate_paths(watch);
    }
  }
done:
  pthread_mutex_unlock(&watch->mutex);
}

static void barrier(void *context) { (void)context; }

typedef struct { slug_fs_watch *watch; slug_producer *reply; } startup;

static void *worker(void *context) {
  startup *start = context;
  slug_fs_watch *watch = start->watch;
  slug_producer *reply = start->reply;
  free(start);
  int error = 0;
  int descriptor = -1;
  dispatch_queue_t queue = NULL;
  FSEventStreamRef stream = NULL;
  bool running = false;
  struct stat before, after;
  if (lstat(watch->input, &before) != 0) { error = errno; goto initialized; }
  if (S_ISLNK(before.st_mode)) { error = ELOOP; goto initialized; }
  if (!S_ISDIR(before.st_mode)) { error = ENOTDIR; goto initialized; }
  watch->root = realpath(watch->input, NULL);
  if (watch->root == NULL) { error = errno; goto initialized; }
  descriptor = open(watch->root, O_EVTONLY | O_NOFOLLOW | O_CLOEXEC);
  if (descriptor < 0) { error = errno; goto initialized; }
  if (fstat(descriptor, &after) != 0) { error = errno; goto initialized; }
  if (before.st_dev != after.st_dev || before.st_ino != after.st_ino) {
    error = ESTALE;
    goto initialized;
  }
  CFStringRef root = CFStringCreateWithCString(NULL, watch->root, kCFStringEncodingUTF8);
  if (root == NULL) { error = EINVAL; goto initialized; }
  CFArrayRef roots = CFArrayCreate(NULL, (const void **)&root, 1, &kCFTypeArrayCallBacks);
  CFRelease(root);
  if (roots == NULL) { error = ENOMEM; goto initialized; }
  FSEventStreamContext info = {0, watch, NULL, NULL, NULL};
  stream = FSEventStreamCreate(NULL, events, &info, roots,
      kFSEventStreamEventIdSinceNow, 0.02,
      kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagWatchRoot |
      kFSEventStreamCreateFlagNoDefer);
  CFRelease(roots);
  if (stream == NULL) { error = ENOMEM; goto initialized; }
  queue = dispatch_queue_create("slug.io.fs.watch", DISPATCH_QUEUE_SERIAL);
  if (queue == NULL) { error = ENOMEM; goto initialized; }
  FSEventStreamSetDispatchQueue(stream, queue);
  running = FSEventStreamStart(stream);
  if (!running) { error = EIO; goto initialized; }
  /* Close the gap between resolving/opening the root and starting its stream. */
  if (lstat(watch->root, &after) != 0 || before.st_dev != after.st_dev ||
      before.st_ino != after.st_ino) error = ESTALE;

initialized:
  pthread_mutex_lock(&watch->mutex);
  if (error != 0) watch->error = error;
  if (watch->error != 0) reply_error(reply, watch->error);
  else {
    slug_producer_item ok = slug_producer_item_i64(SLUG_FS_OK);
    slug_producer_send_list(reply, &ok, 1);
  }
  finish_reply(reply);
  while (!watch->stopped && watch->error == 0 && !slug_producer_is_closed(watch->ready)) {
    /* Only cancellation is checked on a timer. Filesystem changes are never
     * discovered by polling. Resource close wakes this wait immediately. */
    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline);
    deadline.tv_nsec += 100000000;
    if (deadline.tv_nsec >= 1000000000) { deadline.tv_sec++; deadline.tv_nsec -= 1000000000; }
    pthread_cond_timedwait(&watch->condition, &watch->mutex, &deadline);
  }
  watch->stopped = true;
  slug_producer_close(watch->ready);
  pthread_mutex_unlock(&watch->mutex);
  if (running) FSEventStreamStop(stream);
  if (queue != NULL) {
    FSEventStreamInvalidate(stream);
    dispatch_sync_f(queue, NULL, barrier);
  }
  if (stream != NULL) FSEventStreamRelease(stream);
  if (queue != NULL) dispatch_release(queue);
  if (descriptor >= 0) close(descriptor);
  unref(watch);
  return NULL;
}

int32_t slug_fs_watch_create(const char *path, int64_t recursive,
                             slug_producer *ready, slug_fs_watch **result) {
  *result = NULL;
  slug_fs_watch *watch = calloc(1, sizeof(*watch));
  if (watch == NULL) goto failed;
  watch->input = strdup(path);
  if (watch->input == NULL) { free(watch); goto failed; }
  /* A trailing slash must not make lstat follow a final symbolic link. */
  size_t length = strlen(watch->input);
  while (length > 1 && watch->input[length - 1] == '/') watch->input[--length] = 0;
  if (pthread_mutex_init(&watch->mutex, NULL) != 0) {
    free(watch->input); free(watch); goto failed;
  }
  if (pthread_cond_init(&watch->condition, NULL) != 0) {
    pthread_mutex_destroy(&watch->mutex); free(watch->input); free(watch); goto failed;
  }
  atomic_init(&watch->references, 1);
  watch->recursive = recursive != 0;
  watch->ready = ready;
  *result = watch;
  return SLUG_FS_OK;
failed:
  slug_producer_close(ready);
  slug_producer_release(ready);
  return SLUG_FS_ALLOC;
}

int32_t slug_fs_watch_exclude(slug_fs_watch *watch, const char *path) {
  /* Exclusions are configured by the wrapper before starting the worker. */
  size_t length = strlen(path) + 1;
  if (watch->started || watch->excluded_count == WATCH_PATHS ||
      length > WATCH_BYTES - watch->excluded_bytes) return SLUG_FS_ALLOC;
  char *copy = strdup(path);
  if (copy == NULL) return SLUG_FS_ALLOC;
  watch->excluded[watch->excluded_count++] = copy;
  watch->excluded_bytes += length;
  return SLUG_FS_OK;
}

int32_t slug_fs_watch_start(slug_fs_watch *watch, slug_producer *reply) {
  startup *start = malloc(sizeof(*start));
  if (start == NULL) { finish_reply(reply); return SLUG_FS_NOT_QUEUED; }
  *start = (startup){watch, reply};
  watch->started = true;
  atomic_fetch_add(&watch->references, 1);
  pthread_t thread;
  int error = pthread_create(&thread, NULL, worker, start);
  if (error != 0) {
    free(start);
    unref(watch);
    reply_error(reply, error);
    finish_reply(reply);
  } else pthread_detach(thread);
  return 0;
}

int32_t slug_fs_watch_drain(slug_fs_watch *watch, slug_producer *reply) {
  pthread_mutex_lock(&watch->mutex);
  if (watch->error != 0) reply_error(reply, watch->error);
  else if (watch->stopped) {
    slug_producer_item end = slug_producer_item_i64(SLUG_FS_END);
    slug_producer_send_list(reply, &end, 1);
  } else {
    slug_producer_item items[WATCH_PATHS + 2];
    items[0] = slug_producer_item_i64(SLUG_FS_OK);
    items[1] = slug_producer_item_bool(watch->rescan);
    for (size_t i = 0; i < watch->count; i++) {
      items[i + 2] = slug_producer_item_text(watch->paths[i], strlen(watch->paths[i]));
    }
    int32_t status = slug_producer_send_list(reply, items, watch->count + 2);
    if (status == SLUG_PRODUCER_INVALID) {
      /* A filesystem name need not be UTF-8. Losing its spelling requires a
       * rescan, never silent loss or a malformed Slug string. */
      invalidate_paths(watch);
      items[1] = slug_producer_item_bool(true);
      status = slug_producer_send_list(reply, items, 2);
    }
    if (status == SLUG_PRODUCER_SENT) { clear_paths(watch); watch->rescan = false; }
  }
  pthread_mutex_unlock(&watch->mutex);
  finish_reply(reply);
  return 0;
}

void slug_fs_watch_stop(slug_fs_watch *watch) {
  pthread_mutex_lock(&watch->mutex);
  watch->stopped = true;
  clear_paths(watch);
  slug_producer_close(watch->ready);
  pthread_cond_signal(&watch->condition);
  pthread_mutex_unlock(&watch->mutex);
}

void slug_fs_watch_release(slug_fs_watch *watch) {
  slug_fs_watch_stop(watch);
  unref(watch);
}

#else
/* Keep filesystem I/O available on other hosts. Watch has an explicit checked
 * unsupported result until that host has a native event backend. */
struct slug_fs_watch { int unused; };
int32_t slug_fs_watch_create(const char *path, int64_t recursive,
                             slug_producer *ready, slug_fs_watch **watch) {
  (void)path; (void)recursive;
  *watch = NULL;
  slug_producer_close(ready);
  slug_producer_release(ready);
  return 8;
}
int32_t slug_fs_watch_exclude(slug_fs_watch *watch, const char *path) {
  (void)watch; (void)path; return 8;
}
int32_t slug_fs_watch_start(slug_fs_watch *watch, slug_producer *reply) {
  (void)watch;
  slug_producer_close(reply); slug_producer_release(reply); return 1;
}
int32_t slug_fs_watch_drain(slug_fs_watch *watch, slug_producer *reply) {
  return slug_fs_watch_start(watch, reply);
}
void slug_fs_watch_stop(slug_fs_watch *watch) { (void)watch; }
void slug_fs_watch_release(slug_fs_watch *watch) { free(watch); }
#endif

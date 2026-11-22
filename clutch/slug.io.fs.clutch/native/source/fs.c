#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <wchar.h>
#include <windows.h>
#else
/* nftw and mkdtemp are XSI; CLOCK and timespec fields need the rest. Shims
 * build with -std=c11. */
#define _XOPEN_SOURCE 700
#if defined(__APPLE__)
#define _DARWIN_C_SOURCE
#endif
#endif

#include "slug_io_fs.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if !defined(_WIN32)
#include <errno.h>
#include <fcntl.h>
#include <ftw.h>
#include <pthread.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <dirent.h>
#include <unistd.h>
#endif

/* Bounds native memory per line and per read. */
#define MAX_LINE_BYTES ((size_t)16u * 1024u * 1024u)
#define MAX_READ_BYTES ((size_t)16u * 1024u * 1024u)
#define READ_CHUNK ((size_t)64u * 1024u)

/* An open file's system handle, and the value of one that is not open. */
#if defined(_WIN32)
typedef HANDLE fs_handle;
#define NO_HANDLE INVALID_HANDLE_VALUE
#else
typedef int fs_handle;
#define NO_HANDLE (-1)
#endif

struct slug_fs_file {
  /* One for the VM until it releases the file, one per queued job. */
  atomic_int references;
  fs_handle handle;
  /* The mode it was opened in: SLUG_FS_READ, SLUG_FS_WRITE, or
   * SLUG_FS_APPEND. */
  int64_t mode;
  /* Bytes read ahead for read_line, at [start, end) of buffer. */
  uint8_t *buffer;
  size_t start;
  size_t end;
  size_t capacity;
};

typedef enum {
  JOB_OPEN,
  JOB_READ,
  JOB_READ_LINE,
  JOB_WRITE,
  JOB_SYNC,
  JOB_CLOSE,
  JOB_STAT,
  JOB_LIST,
  JOB_MAKE_DIRECTORY,
  JOB_RENAME,
  JOB_REMOVE,
  JOB_REMOVE_ALL,
  JOB_TEMPORARY_DIRECTORY,
} job_kind;

typedef struct job {
  job_kind kind;
  slug_fs_file *file;
  char *path;
  char *other;
  int64_t number;
  uint8_t *data;
  size_t length;
  slug_producer *reply;
  struct job *next;
} job;

static job *queue_head = NULL;
static job *queue_tail = NULL;
static bool started = false;

/* ---- Files ---------------------------------------------------------- */

static void file_unref(slug_fs_file *file) {
  if (atomic_fetch_sub(&file->references, 1) == 1) {
    free(file->buffer);
    free(file);
  }
}

/* ---- Failures ------------------------------------------------------- */

/* A failure's portable code and its description, for the system's own
 * error number: errno, or a Windows error code. */
typedef struct {
  const char *code;
  const char *message;
} failure;

#if defined(_WIN32)

/* Not in every SDK's headers. */
#ifndef ERROR_DIRECTORY_NOT_SUPPORTED
#define ERROR_DIRECTORY_NOT_SUPPORTED 336L
#endif
#ifndef ERROR_CANT_RESOLVE_FILENAME
#define ERROR_CANT_RESOLVE_FILENAME 1921L
#endif

static failure describe(int error) {
  switch (error) {
    case ERROR_FILE_NOT_FOUND:
    case ERROR_PATH_NOT_FOUND:
    case ERROR_INVALID_DRIVE:
    case ERROR_BAD_NETPATH:
    case ERROR_BAD_NET_NAME: return (failure){"not_found", "no such file or directory"};
    case ERROR_FILE_EXISTS:
    case ERROR_ALREADY_EXISTS: return (failure){"exists", "it already exists"};
    case ERROR_ACCESS_DENIED:
    case ERROR_PRIVILEGE_NOT_HELD: return (failure){"permission", "permission denied"};
    case ERROR_DIRECTORY:
      return (failure){"not_directory", "a part of the path is not a directory"};
    case ERROR_DIRECTORY_NOT_SUPPORTED: return (failure){"is_directory", "it is a directory"};
    case ERROR_DIR_NOT_EMPTY: return (failure){"not_empty", "the directory is not empty"};
    case ERROR_NOT_SAME_DEVICE: return (failure){"cross_device", "it is on another filesystem"};
    case ERROR_DISK_FULL:
    case ERROR_HANDLE_DISK_FULL: return (failure){"no_space", "no space left on the device"};
    case ERROR_WRITE_PROTECT: return (failure){"read_only", "the filesystem is read-only"};
    case ERROR_TOO_MANY_OPEN_FILES: return (failure){"too_many_open", "too many files are open"};
    case ERROR_FILENAME_EXCED_RANGE:
    case ERROR_BUFFER_OVERFLOW: return (failure){"name_too_long", "the name is too long"};
    case ERROR_CANT_RESOLVE_FILENAME:
      return (failure){"link_loop", "too many symbolic links"};
    case ERROR_INVALID_HANDLE: return (failure){"wrong_mode", "the file is not open for that"};
    case ERROR_INVALID_NAME:
    case ERROR_INVALID_PARAMETER:
    case ERROR_NO_UNICODE_TRANSLATION: return (failure){"invalid", "invalid argument"};
    case ERROR_SHARING_VIOLATION:
    case ERROR_LOCK_VIOLATION:
    case ERROR_BUSY: return (failure){"busy", "it is in use"};
    case ERROR_IO_DEVICE:
    case ERROR_CRC: return (failure){"io", "input/output error"};
    default: return (failure){"io", "the system reported an error"};
  }
}

/* The calling thread's last error, as a failure's number. */
static int last_error(void) { return (int)GetLastError(); }

#else

static failure describe(int error) {
  switch (error) {
    case ENOENT: return (failure){"not_found", "no such file or directory"};
    case EEXIST: return (failure){"exists", "it already exists"};
    case EACCES:
    case EPERM: return (failure){"permission", "permission denied"};
    case ENOTDIR: return (failure){"not_directory", "a part of the path is not a directory"};
    case EISDIR: return (failure){"is_directory", "it is a directory"};
    case ENOTEMPTY: return (failure){"not_empty", "the directory is not empty"};
    case EXDEV: return (failure){"cross_device", "it is on another filesystem"};
    case ENOSPC: return (failure){"no_space", "no space left on the device"};
    case EROFS: return (failure){"read_only", "the filesystem is read-only"};
    case EMFILE:
    case ENFILE: return (failure){"too_many_open", "too many files are open"};
    case ENAMETOOLONG: return (failure){"name_too_long", "the name is too long"};
    case ELOOP: return (failure){"link_loop", "too many symbolic links"};
    case EBADF: return (failure){"wrong_mode", "the file is not open for that"};
    case EINVAL: return (failure){"invalid", "invalid argument"};
    case EBUSY: return (failure){"busy", "it is in use"};
    case EIO: return (failure){"io", "input/output error"};
    default: return (failure){"io", "the system reported an error"};
  }
}


#endif

/* The failure number for an argument that is not valid, as a name with a
 * separator in it. */
#if defined(_WIN32)
#define INVALID_ARGUMENT ((int)ERROR_INVALID_PARAMETER)
#define WRONG_MODE ((int)ERROR_INVALID_HANDLE)
#else
#define INVALID_ARGUMENT EINVAL
#define WRONG_MODE EBADF
#endif

/* ---- Replies -------------------------------------------------------- */

static void reply_items(job *work, const slug_producer_item *items, size_t count) {
  slug_producer_send_list(work->reply, items, count);
}

static void reply_status(job *work, int32_t status) {
  slug_producer_item item = slug_producer_item_i64(status);
  reply_items(work, &item, 1);
}

static void reply_system(job *work, int error) {
  failure described = describe(error);
  slug_producer_item items[4];
  items[0] = slug_producer_item_i64(SLUG_FS_SYSTEM);
  items[1] = slug_producer_item_i64(error);
  items[2] = slug_producer_item_text(described.code, strlen(described.code));
  items[3] = slug_producer_item_text(described.message, strlen(described.message));
  reply_items(work, items, 4);
}

static void reply_number(job *work, int64_t value) {
  slug_producer_item items[2];
  items[0] = slug_producer_item_i64(SLUG_FS_OK);
  items[1] = slug_producer_item_i64(value);
  reply_items(work, items, 2);
}

static void reply_text(job *work, const char *text, size_t length) {
  slug_producer_item items[2];
  items[0] = slug_producer_item_i64(SLUG_FS_OK);
  items[1] = slug_producer_item_text(text, length);
  if (slug_producer_send_list(work->reply, items, 2) == SLUG_PRODUCER_INVALID) {
    reply_status(work, SLUG_FS_LINE_NOT_UTF8);
  }
}

static void reply_kind(job *work, const char *kind, int64_t size, int64_t modified) {
  slug_producer_item items[4];
  items[0] = slug_producer_item_i64(SLUG_FS_OK);
  items[1] = slug_producer_item_text(kind, strlen(kind));
  items[2] = slug_producer_item_i64(size);
  items[3] = slug_producer_item_i64(modified);
  reply_items(work, items, 4);
}

/* ---- Directory listings --------------------------------------------- */

/* Names gathered for a listing, which it sorts and sends. */
typedef struct {
  char **names;
  size_t count;
  size_t capacity;
} names;

/* Adds a copy of `length` bytes of `name`; false when memory ran out. */
static bool add_name(names *list, const char *name, size_t length) {
  char *copy;
  if (list->count == list->capacity) {
    size_t grown_capacity = list->capacity == 0 ? 16 : list->capacity * 2;
    char **grown = realloc(list->names, grown_capacity * sizeof *grown);
    if (grown == NULL) return false;
    list->names = grown;
    list->capacity = grown_capacity;
  }
  copy = malloc(length + 1);
  if (copy == NULL) return false;
  memcpy(copy, name, length);
  copy[length] = '\0';
  list->names[list->count++] = copy;
  return true;
}

static void free_names(names *list) {
  size_t i;
  for (i = 0; i < list->count; i++) free(list->names[i]);
  free(list->names);
}

static int compare_names(const void *left, const void *right) {
  return strcmp(*(char *const *)left, *(char *const *)right);
}

/* Sends the names, sorted by byte value. */
static void reply_names(job *work, names *list) {
  slug_producer_item *items = malloc((list->count + 1) * sizeof *items);
  size_t i;
  if (items == NULL) {
    reply_status(work, SLUG_FS_ALLOC);
    return;
  }
  qsort(list->names, list->count, sizeof *list->names, compare_names);
  items[0] = slug_producer_item_i64(SLUG_FS_OK);
  for (i = 0; i < list->count; i++) {
    items[i + 1] = slug_producer_item_text(list->names[i], strlen(list->names[i]));
  }
  if (slug_producer_send_list(work->reply, items, list->count + 1) == SLUG_PRODUCER_INVALID) {
    /* A name that is not UTF-8. */
    reply_system(work, INVALID_ARGUMENT);
  }
  free(items);
}

#if defined(_WIN32)

/* ---- Windows: text and paths ---------------------------------------- */

/* `text`, UTF-8, as a new UTF-16 string, or NULL with the thread's last
 * error set: ERROR_NO_UNICODE_TRANSLATION for text that is not UTF-8. */
static wchar_t *wide(const char *text) {
  int length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text, -1, NULL, 0);
  wchar_t *converted;
  if (length <= 0) return NULL;
  converted = malloc((size_t)length * sizeof *converted);
  if (converted == NULL) {
    SetLastError(ERROR_NOT_ENOUGH_MEMORY);
    return NULL;
  }
  if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text, -1, converted, length) <= 0) {
    free(converted);
    return NULL;
  }
  return converted;
}

/* `text`, UTF-16, as a new UTF-8 string, or NULL when it holds an unpaired
 * surrogate or memory ran out. */
static char *narrow(const wchar_t *text) {
  int length =
      WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text, -1, NULL, 0, NULL, NULL);
  char *converted;
  if (length <= 0) return NULL;
  converted = malloc((size_t)length);
  if (converted == NULL) return NULL;
  if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text, -1, converted, length, NULL,
                          NULL) <= 0) {
    free(converted);
    return NULL;
  }
  return converted;
}

static bool is_separator(wchar_t character) { return character == L'/' || character == L'\\'; }

/* `directory` and `name` joined by a backslash, as a new string, or NULL. */
static wchar_t *child_path(const wchar_t *directory, const wchar_t *name) {
  size_t directory_length = wcslen(directory);
  size_t name_length = wcslen(name);
  bool separated = directory_length > 0 && is_separator(directory[directory_length - 1]);
  size_t length = directory_length + (separated ? 0 : 1) + name_length;
  wchar_t *path = malloc((length + 1) * sizeof *path);
  if (path == NULL) return NULL;
  memcpy(path, directory, directory_length * sizeof *path);
  if (!separated) path[directory_length] = L'\\';
  memcpy(path + length - name_length, name, (name_length + 1) * sizeof *path);
  return path;
}

/* Whether `path` begins with `prefix`. */
static bool starts_with(const wchar_t *path, const wchar_t *prefix) {
  return wcsncmp(path, prefix, wcslen(prefix)) == 0;
}

/* `text`, a UTF-8 path, as a new UTF-16 path in the extended form that has
 * no MAX_PATH limit: made absolute first, as Windows resolves a path, with
 * `/` as `\`, `.` and `..` resolved, and a relative path taken from the
 * current directory, then prefixed with `\\?\` (`\\?\UNC\` for a share). A
 * device such as `NUL`, and a path already in that form, are left as they
 * are. NULL with the thread's last error set when it cannot be made. */
static wchar_t *wide_path(const char *text) {
  wchar_t *given = wide(text);
  wchar_t *full;
  wchar_t *extended;
  DWORD length;
  DWORD written;
  size_t full_length;
  bool share;
  if (given == NULL) return NULL;
  if (given[0] == L'\0') {
    free(given);
    SetLastError(ERROR_PATH_NOT_FOUND);
    return NULL;
  }
  if (starts_with(given, L"\\\\?\\") || starts_with(given, L"\\\\.\\")) return given;
  length = GetFullPathNameW(given, 0, NULL, NULL);
  if (length == 0) {
    free(given);
    return NULL;
  }
  full = malloc((size_t)length * sizeof *full);
  if (full == NULL) {
    free(given);
    SetLastError(ERROR_NOT_ENOUGH_MEMORY);
    return NULL;
  }
  written = GetFullPathNameW(given, length, full, NULL);
  free(given);
  if (written == 0 || written >= length) {
    free(full);
    if (written != 0) SetLastError(ERROR_FILENAME_EXCED_RANGE);
    return NULL;
  }
  /* A device resolves to its own form. */
  if (starts_with(full, L"\\\\.\\") || starts_with(full, L"\\\\?\\")) return full;
  full_length = written;
  share = full_length >= 2 && full[0] == L'\\' && full[1] == L'\\';
  {
    const wchar_t *prefix = share ? L"\\\\?\\UNC" : L"\\\\?\\";
    size_t prefix_length = wcslen(prefix);
    /* A share keeps one of its two leading backslashes, after "UNC". */
    const wchar_t *rest = share ? full + 1 : full;
    size_t rest_length = share ? full_length - 1 : full_length;
    extended = malloc((prefix_length + rest_length + 1) * sizeof *extended);
    if (extended == NULL) {
      free(full);
      SetLastError(ERROR_NOT_ENOUGH_MEMORY);
      return NULL;
    }
    memcpy(extended, prefix, prefix_length * sizeof *extended);
    memcpy(extended + prefix_length, rest, (rest_length + 1) * sizeof *extended);
  }
  free(full);
  return extended;
}

/* The failure number for a conversion that failed: the thread's last error,
 * or an invalid argument when there was none. */
static int conversion_error(void) {
  int error = last_error();
  return error != 0 ? error : INVALID_ARGUMENT;
}

/* Milliseconds since the Unix epoch of a file time, in 100 ns ticks since
 * 1601. */
static int64_t milliseconds(FILETIME time) {
  uint64_t ticks = ((uint64_t)time.dwHighDateTime << 32) | time.dwLowDateTime;
  return ((int64_t)ticks - 116444736000000000LL) / 10000;
}

/* Whether the reparse point at `path` is a symbolic link or a junction,
 * which `stat` reports as a link; other reparse points, such as cloud
 * placeholders, are what they hold. */
static bool is_link(const wchar_t *path) {
  WIN32_FIND_DATAW found;
  HANDLE search = FindFirstFileW(path, &found);
  if (search == INVALID_HANDLE_VALUE) return true;
  FindClose(search);
  return found.dwReserved0 == IO_REPARSE_TAG_SYMLINK ||
         found.dwReserved0 == IO_REPARSE_TAG_MOUNT_POINT;
}

/* ---- Windows: file work --------------------------------------------- */

static void run_open(job *work) {
  DWORD access;
  DWORD disposition;
  wchar_t *path = wide_path(work->path);
  HANDLE handle;
  if (path == NULL) {
    reply_system(work, conversion_error());
    return;
  }
  switch (work->number) {
    case SLUG_FS_WRITE:
      access = GENERIC_WRITE;
      disposition = CREATE_ALWAYS;
      break;
    case SLUG_FS_APPEND:
      access = FILE_APPEND_DATA | SYNCHRONIZE;
      disposition = OPEN_ALWAYS;
      break;
    default:
      access = GENERIC_READ;
      disposition = OPEN_EXISTING;
      break;
  }
  /* Shared as POSIX files are: others may read, write, rename, and remove
   * it while it is open. Not inherited by child processes. */
  handle = CreateFileW(path, access, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                       NULL, disposition, FILE_ATTRIBUTE_NORMAL, NULL);
  if (handle == INVALID_HANDLE_VALUE) {
    int error = last_error();
    DWORD attributes = GetFileAttributesW(path);
    free(path);
    /* A directory cannot be opened as a file. */
    if (error == (int)ERROR_ACCESS_DENIED && attributes != INVALID_FILE_ATTRIBUTES &&
        (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
      error = (int)ERROR_DIRECTORY_NOT_SUPPORTED;
    }
    reply_system(work, error);
    return;
  }
  free(path);
  work->file->handle = handle;
  reply_status(work, SLUG_FS_OK);
}

/* Reads into the file's buffer after `end`, as `fill` does below. */
static int64_t read_more(slug_fs_file *file, size_t wanted, int *error) {
  DWORD count = 0;
  if (file->mode != SLUG_FS_READ) {
    *error = WRONG_MODE;
    return -1;
  }
  if (!ReadFile(file->handle, file->buffer + file->end, (DWORD)wanted, &count, NULL)) {
    *error = last_error();
    /* A pipe's writer closing is its end. */
    if (*error == (int)ERROR_BROKEN_PIPE) return 0;
    return -1;
  }
  return (int64_t)count;
}

static void run_write(job *work) {
  size_t written = 0;
  if (work->file->mode == SLUG_FS_READ) {
    reply_system(work, WRONG_MODE);
    return;
  }
  while (written < work->length) {
    size_t left = work->length - written;
    DWORD count = 0;
    DWORD chunk = left > (size_t)0x40000000u ? (DWORD)0x40000000u : (DWORD)left;
    if (!WriteFile(work->file->handle, work->data + written, chunk, &count, NULL)) {
      reply_system(work, last_error());
      return;
    }
    written += count;
  }
  reply_number(work, (int64_t)written);
}

static void run_sync(job *work) {
  /* Nothing written through a file opened for reading needs flushing, and
   * Windows refuses to flush one. */
  if (work->file->mode != SLUG_FS_READ && !FlushFileBuffers(work->file->handle)) {
    reply_system(work, last_error());
    return;
  }
  reply_status(work, SLUG_FS_OK);
}

static void run_close(job *work) {
  if (work->file->handle != NO_HANDLE) CloseHandle(work->file->handle);
  work->file->handle = NO_HANDLE;
}

/* ---- Windows: path work --------------------------------------------- */

static bool is_missing(int error) {
  return error == (int)ERROR_FILE_NOT_FOUND || error == (int)ERROR_PATH_NOT_FOUND ||
         error == (int)ERROR_DIRECTORY || error == (int)ERROR_INVALID_DRIVE ||
         error == (int)ERROR_BAD_NETPATH || error == (int)ERROR_BAD_NET_NAME;
}

static void run_stat(job *work) {
  WIN32_FILE_ATTRIBUTE_DATA info;
  const char *kind;
  wchar_t *path = wide_path(work->path);
  if (path == NULL) {
    int error = conversion_error();
    if (is_missing(error)) {
      reply_status(work, SLUG_FS_END);
      return;
    }
    reply_system(work, error);
    return;
  }
  /* The attributes of a final link itself, not of what it points to. */
  if (!GetFileAttributesExW(path, GetFileExInfoStandard, &info)) {
    int error = last_error();
    free(path);
    if (is_missing(error)) {
      reply_status(work, SLUG_FS_END);
      return;
    }
    reply_system(work, error);
    return;
  }
  if ((info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 && is_link(path)) {
    kind = "link";
  } else if ((info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
    kind = "directory";
  } else if ((info.dwFileAttributes & FILE_ATTRIBUTE_DEVICE) != 0) {
    kind = "other";
  } else {
    kind = "file";
  }
  free(path);
  reply_kind(work, kind,
             (int64_t)(((uint64_t)info.nFileSizeHigh << 32) | info.nFileSizeLow),
             milliseconds(info.ftLastWriteTime));
}

static void run_list(job *work) {
  wchar_t *path = wide_path(work->path);
  wchar_t *pattern;
  WIN32_FIND_DATAW found;
  HANDLE search;
  DWORD attributes;
  names list = {NULL, 0, 0};
  int error = 0;
  if (path == NULL) {
    reply_system(work, conversion_error());
    return;
  }
  attributes = GetFileAttributesW(path);
  if (attributes == INVALID_FILE_ATTRIBUTES) {
    error = last_error();
    free(path);
    reply_system(work, error);
    return;
  }
  if ((attributes & FILE_ATTRIBUTE_DIRECTORY) == 0) {
    free(path);
    reply_system(work, (int)ERROR_DIRECTORY);
    return;
  }
  pattern = child_path(path, L"*");
  free(path);
  if (pattern == NULL) {
    reply_status(work, SLUG_FS_ALLOC);
    return;
  }
  search = FindFirstFileW(pattern, &found);
  free(pattern);
  if (search == INVALID_HANDLE_VALUE) {
    error = last_error();
    /* An empty drive's root has no entries at all, not even ".". */
    if (error != (int)ERROR_FILE_NOT_FOUND) {
      reply_system(work, error);
      return;
    }
    error = 0;
  } else {
    do {
      char *name;
      if (wcscmp(found.cFileName, L".") == 0 || wcscmp(found.cFileName, L"..") == 0) continue;
      name = narrow(found.cFileName);
      if (name == NULL) {
        error = INVALID_ARGUMENT;
        break;
      }
      if (!add_name(&list, name, strlen(name))) {
        free(name);
        error = -1;
        break;
      }
      free(name);
    } while (FindNextFileW(search, &found));
    if (error == 0 && GetLastError() != ERROR_NO_MORE_FILES) error = last_error();
    FindClose(search);
  }
  if (error == 0) {
    reply_names(work, &list);
  } else if (error < 0) {
    reply_status(work, SLUG_FS_ALLOC);
  } else {
    reply_system(work, error);
  }
  free_names(&list);
}

static bool is_directory(const wchar_t *path) {
  DWORD attributes = GetFileAttributesW(path);
  return attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
}

/* Makes `path`; an existing directory is not a failure when `existing`. */
static int make_one(const wchar_t *path, bool existing) {
  if (CreateDirectoryW(path, NULL)) return 0;
  {
    int error = last_error();
    if (existing && error == (int)ERROR_ALREADY_EXISTS && is_directory(path)) return 0;
    return error;
  }
}

/* The length of `path`'s root, which is never made: a drive ("C:" or
 * "C:\"), a UNC share ("\\server\share\"), or a leading separator, each
 * after the extended form's "\\?\" or "\\?\UNC\". */
static size_t root_length(const wchar_t *path) {
  size_t length = 0;
  if (starts_with(path, L"\\\\?\\UNC\\")) {
    /* Past the server and the share. */
    int parts = 0;
    length = 8;
    while (path[length] != L'\0' && parts < 2) {
      if (is_separator(path[length])) parts++;
      length++;
    }
    return length;
  }
  if (starts_with(path, L"\\\\?\\")) {
    size_t rest = root_length(path + 4);
    return 4 + rest;
  }
  if (path[0] != L'\0' && path[1] == L':') {
    length = 2;
  } else if (is_separator(path[0]) && is_separator(path[1])) {
    /* Past the server and the share. */
    int parts = 0;
    length = 2;
    while (path[length] != L'\0' && parts < 2) {
      if (is_separator(path[length])) parts++;
      length++;
    }
    return length;
  }
  while (is_separator(path[length])) length++;
  return length;
}

static void run_make_directory(job *work) {
  wchar_t *path = wide_path(work->path);
  int error;
  if (path == NULL) {
    reply_system(work, conversion_error());
    return;
  }
  if (work->number != 0) {
    /* Each parent in turn, then the directory itself. */
    wchar_t *cursor = path + root_length(path);
    for (; *cursor != L'\0'; cursor++) {
      wchar_t separator = *cursor;
      if (!is_separator(separator)) continue;
      *cursor = L'\0';
      error = make_one(path, true);
      *cursor = separator;
      if (error != 0) {
        free(path);
        reply_system(work, error);
        return;
      }
    }
  }
  error = make_one(path, work->number != 0);
  free(path);
  if (error != 0) {
    reply_system(work, error);
    return;
  }
  reply_status(work, SLUG_FS_OK);
}

static void run_rename(job *work) {
  wchar_t *from = wide_path(work->path);
  wchar_t *to = from == NULL ? NULL : wide_path(work->other);
  int error = 0;
  if (from == NULL || to == NULL) {
    error = conversion_error();
  } else if (!MoveFileExW(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    error = last_error();
  }
  free(from);
  free(to);
  if (error != 0) {
    reply_system(work, error);
    return;
  }
  reply_status(work, SLUG_FS_OK);
}

/* Removes the file or link at `path`, whose attributes are `attributes`,
 * or the empty directory; a read-only file too, as POSIX allows. */
static int remove_entry(const wchar_t *path, DWORD attributes) {
  if ((attributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
    /* A directory link too, which leaves what it points to. */
    return RemoveDirectoryW(path) ? 0 : last_error();
  }
  if (DeleteFileW(path)) return 0;
  {
    int error = last_error();
    if (error == (int)ERROR_ACCESS_DENIED && (attributes & FILE_ATTRIBUTE_READONLY) != 0 &&
        SetFileAttributesW(path, attributes & ~(DWORD)FILE_ATTRIBUTE_READONLY)) {
      return DeleteFileW(path) ? 0 : last_error();
    }
    return error;
  }
}

static void run_remove(job *work) {
  wchar_t *path = wide_path(work->path);
  DWORD attributes;
  int error;
  if (path == NULL) {
    reply_system(work, conversion_error());
    return;
  }
  attributes = GetFileAttributesW(path);
  error = attributes == INVALID_FILE_ATTRIBUTES ? last_error() : remove_entry(path, attributes);
  free(path);
  if (error != 0) {
    reply_system(work, error);
    return;
  }
  reply_status(work, SLUG_FS_OK);
}

/* Removes `path` and everything under it, never following a link; what is
 * already gone is not a failure. Returns 0, a failure number, or -1 when
 * memory ran out. */
static int remove_tree(const wchar_t *path, DWORD attributes) {
  int error = 0;
  if ((attributes & FILE_ATTRIBUTE_DIRECTORY) != 0 &&
      (attributes & FILE_ATTRIBUTE_REPARSE_POINT) == 0) {
    WIN32_FIND_DATAW found;
    wchar_t *pattern = child_path(path, L"*");
    HANDLE search;
    if (pattern == NULL) return -1;
    search = FindFirstFileW(pattern, &found);
    free(pattern);
    if (search != INVALID_HANDLE_VALUE) {
      do {
        wchar_t *child;
        if (wcscmp(found.cFileName, L".") == 0 || wcscmp(found.cFileName, L"..") == 0) continue;
        child = child_path(path, found.cFileName);
        if (child == NULL) {
          error = -1;
          break;
        }
        error = remove_tree(child, found.dwFileAttributes);
        free(child);
      } while (error == 0 && FindNextFileW(search, &found));
      FindClose(search);
      if (error != 0) return error;
    }
  }
  error = remove_entry(path, attributes);
  return is_missing(error) ? 0 : error;
}

static void run_remove_all(job *work) {
  wchar_t *path = wide_path(work->path);
  DWORD attributes;
  int error;
  if (path == NULL) {
    error = conversion_error();
    if (is_missing(error)) {
      reply_status(work, SLUG_FS_OK);
      return;
    }
    reply_system(work, error);
    return;
  }
  attributes = GetFileAttributesW(path);
  if (attributes == INVALID_FILE_ATTRIBUTES) {
    error = last_error();
    free(path);
    if (is_missing(error)) {
      reply_status(work, SLUG_FS_OK);
      return;
    }
    reply_system(work, error);
    return;
  }
  error = remove_tree(path, attributes);
  free(path);
  if (error < 0) {
    reply_status(work, SLUG_FS_ALLOC);
  } else if (error != 0) {
    reply_system(work, error);
  } else {
    reply_status(work, SLUG_FS_OK);
  }
}

/* A generator for temporary names: they need to be unlikely to collide, not
 * to be guessed, since each user's temporary directory is their own. */
static uint64_t next_random(uint64_t *state) {
  *state ^= *state << 13;
  *state ^= *state >> 7;
  *state ^= *state << 17;
  return *state;
}

static void run_temporary_directory(job *work) {
  static const char letters[] = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  wchar_t base[MAX_PATH + 1];
  wchar_t *prefix;
  wchar_t *path;
  size_t base_length;
  size_t prefix_length;
  LARGE_INTEGER counter;
  uint64_t state;
  int attempt;
  int error = (int)ERROR_ALREADY_EXISTS;
  DWORD got = GetTempPathW(MAX_PATH + 1, base);
  if (strchr(work->path, '/') != NULL || strchr(work->path, '\\') != NULL) {
    reply_system(work, INVALID_ARGUMENT);
    return;
  }
  if (got == 0 || got > MAX_PATH) {
    reply_system(work, got == 0 ? last_error() : (int)ERROR_FILENAME_EXCED_RANGE);
    return;
  }
  prefix = wide(work->path);
  if (prefix == NULL) {
    reply_system(work, conversion_error());
    return;
  }
  base_length = got;
  prefix_length = wcslen(prefix);
  path = malloc((base_length + prefix_length + 7) * sizeof *path);
  if (path == NULL) {
    free(prefix);
    reply_status(work, SLUG_FS_ALLOC);
    return;
  }
  memcpy(path, base, base_length * sizeof *path);
  memcpy(path + base_length, prefix, prefix_length * sizeof *path);
  free(prefix);
  QueryPerformanceCounter(&counter);
  state = (uint64_t)counter.QuadPart ^ ((uint64_t)GetCurrentProcessId() << 32) ^
          (uint64_t)GetCurrentThreadId();
  if (state == 0) state = 0x9e3779b97f4a7c15ULL;
  for (attempt = 0; attempt < 100 && error == (int)ERROR_ALREADY_EXISTS; attempt++) {
    size_t i;
    for (i = 0; i < 6; i++) {
      path[base_length + prefix_length + i] =
          (wchar_t)letters[next_random(&state) % (sizeof letters - 1)];
    }
    path[base_length + prefix_length + 6] = L'\0';
    error = CreateDirectoryW(path, NULL) ? 0 : last_error();
  }
  if (error != 0) {
    free(path);
    reply_system(work, error);
    return;
  }
  {
    char *made = narrow(path);
    free(path);
    if (made == NULL) {
      reply_status(work, SLUG_FS_ALLOC);
      return;
    }
    reply_text(work, made, strlen(made));
    free(made);
  }
}

#else

/* ---- POSIX: file work ----------------------------------------------- */

static void run_open(job *work) {
  int flags;
  int descriptor;
  switch (work->number) {
    case SLUG_FS_WRITE: flags = O_WRONLY | O_CREAT | O_TRUNC; break;
    case SLUG_FS_APPEND: flags = O_WRONLY | O_CREAT | O_APPEND; break;
    default: flags = O_RDONLY; break;
  }
  do {
    descriptor = open(work->path, flags | O_CLOEXEC, 0666);
  } while (descriptor < 0 && errno == EINTR);
  if (descriptor < 0) {
    reply_system(work, errno);
    return;
  }
  work->file->handle = descriptor;
  reply_status(work, SLUG_FS_OK);
}

/* Reads into the file's buffer after `end`, as `fill` does below. */
static int64_t read_more(slug_fs_file *file, size_t wanted, int *error) {
  ssize_t count;
  do {
    count = read(file->handle, file->buffer + file->end, wanted);
  } while (count < 0 && errno == EINTR);
  if (count < 0) *error = errno;
  return (int64_t)count;
}

static void run_write(job *work) {
  size_t written = 0;
  while (written < work->length) {
    ssize_t count = write(work->file->handle, work->data + written, work->length - written);
    if (count < 0) {
      if (errno == EINTR) continue;
      { reply_system(work, errno); return; }
    }
    written += (size_t)count;
  }
  reply_number(work, (int64_t)written);
}

static void run_sync(job *work) {
  if (fsync(work->file->handle) != 0) { reply_system(work, errno); return; }
  reply_status(work, SLUG_FS_OK);
}

static void run_close(job *work) {
  if (work->file->handle >= 0) close(work->file->handle);
  work->file->handle = -1;
}

/* ---- POSIX: path work ----------------------------------------------- */

static int64_t milliseconds(const struct stat *info) {
#if defined(__APPLE__)
  return (int64_t)info->st_mtimespec.tv_sec * 1000 + info->st_mtimespec.tv_nsec / 1000000;
#else
  return (int64_t)info->st_mtim.tv_sec * 1000 + info->st_mtim.tv_nsec / 1000000;
#endif
}

static void run_stat(job *work) {
  struct stat info;
  const char *kind;
  if (lstat(work->path, &info) != 0) {
    if (errno == ENOENT || errno == ENOTDIR) { reply_status(work, SLUG_FS_END); return; }
    { reply_system(work, errno); return; }
  }
  if (S_ISREG(info.st_mode)) {
    kind = "file";
  } else if (S_ISDIR(info.st_mode)) {
    kind = "directory";
  } else if (S_ISLNK(info.st_mode)) {
    kind = "link";
  } else {
    kind = "other";
  }
  reply_kind(work, kind, (int64_t)info.st_size, milliseconds(&info));
}

static void run_list(job *work) {
  DIR *directory = opendir(work->path);
  struct dirent *entry;
  names list = {NULL, 0, 0};
  int failure_number = 0;
  if (directory == NULL) { reply_system(work, errno); return; }
  for (;;) {
    errno = 0;
    entry = readdir(directory);
    if (entry == NULL) {
      failure_number = errno;
      break;
    }
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
    if (!add_name(&list, entry->d_name, strlen(entry->d_name))) {
      failure_number = -1;
      break;
    }
  }
  closedir(directory);
  if (failure_number == 0) {
    reply_names(work, &list);
  } else if (failure_number < 0) {
    reply_status(work, SLUG_FS_ALLOC);
  } else {
    reply_system(work, failure_number);
  }
  free_names(&list);
}

static bool is_directory(const char *path) {
  struct stat info;
  return stat(path, &info) == 0 && S_ISDIR(info.st_mode);
}

static void run_make_directory(job *work) {
  if (work->number != 0) {
    /* Each parent in turn, then the directory itself. */
    char *cursor = work->path;
    while (*cursor == '/') cursor++;
    for (; *cursor != '\0'; cursor++) {
      if (*cursor != '/') continue;
      *cursor = '\0';
      if (mkdir(work->path, 0777) != 0 && !(errno == EEXIST && is_directory(work->path))) {
        int error = errno;
        *cursor = '/';
        { reply_system(work, error); return; }
      }
      *cursor = '/';
    }
  }
  if (mkdir(work->path, 0777) != 0 &&
      !(work->number != 0 && errno == EEXIST && is_directory(work->path))) {
    { reply_system(work, errno); return; }
  }
  reply_status(work, SLUG_FS_OK);
}

static void run_rename(job *work) {
  if (rename(work->path, work->other) != 0) { reply_system(work, errno); return; }
  reply_status(work, SLUG_FS_OK);
}

static void run_remove(job *work) {
  if (remove(work->path) != 0) { reply_system(work, errno); return; }
  reply_status(work, SLUG_FS_OK);
}

/* nftw takes no context; only the I/O thread walks. */
static int remove_failure = 0;

static int remove_walked(const char *path, const struct stat *info, int kind,
                         struct FTW *walk) {
  (void)info;
  (void)walk;
  if (kind == FTW_NS && errno == ENOENT) return 0;
  if (remove(path) != 0 && errno != ENOENT) {
    remove_failure = errno;
    return 1;
  }
  return 0;
}

static void run_remove_all(job *work) {
  struct stat info;
  if (lstat(work->path, &info) != 0) {
    if (errno == ENOENT) { reply_status(work, SLUG_FS_OK); return; }
    { reply_system(work, errno); return; }
  }
  remove_failure = 0;
  if (nftw(work->path, remove_walked, 16, FTW_DEPTH | FTW_PHYS) != 0) {
    { reply_system(work, remove_failure != 0 ? remove_failure : errno); return; }
  }
  reply_status(work, SLUG_FS_OK);
}

static void run_temporary_directory(job *work) {
  const char *base = getenv("TMPDIR");
  size_t base_length;
  size_t prefix_length = strlen(work->path);
  char *template;
  if (base == NULL || *base == '\0') base = "/tmp";
  base_length = strlen(base);
  while (base_length > 1 && base[base_length - 1] == '/') base_length--;
  if (strchr(work->path, '/') != NULL) { reply_system(work, EINVAL); return; }
  template = malloc(base_length + 1 + prefix_length + 7);
  if (template == NULL) { reply_status(work, SLUG_FS_ALLOC); return; }
  memcpy(template, base, base_length);
  template[base_length] = '/';
  memcpy(template + base_length + 1, work->path, prefix_length);
  memcpy(template + base_length + 1 + prefix_length, "XXXXXX", 7);
  if (mkdtemp(template) == NULL) {
    int error = errno;
    free(template);
    { reply_system(work, error); return; }
  }
  reply_text(work, template, strlen(template));
  free(template);
}

#endif

/* ---- Reading, on either system -------------------------------------- */

/* Reads into the file's buffer after `end`, growing it to hold `wanted` more
 * bytes. Returns the count read, 0 at end of file, -1 with `error` set, or
 * -2 when memory ran out. */
static int64_t fill(slug_fs_file *file, size_t wanted, int *error) {
  int64_t count;
  if (file->start > 0 && file->start == file->end) file->start = file->end = 0;
  if (file->capacity - file->end < wanted) {
    if (file->start > 0) {
      memmove(file->buffer, file->buffer + file->start, file->end - file->start);
      file->end -= file->start;
      file->start = 0;
    }
    if (file->capacity - file->end < wanted) {
      size_t capacity = file->end + wanted;
      uint8_t *grown = realloc(file->buffer, capacity);
      if (grown == NULL) return -2;
      file->buffer = grown;
      file->capacity = capacity;
    }
  }
  count = read_more(file, wanted, error);
  if (count > 0) file->end += (size_t)count;
  return count;
}

static void run_read(job *work) {
  slug_fs_file *file = work->file;
  size_t wanted = (size_t)work->number;
  slug_producer_item items[2];
  int error = 0;
  if (file->start == file->end) {
    int64_t count = fill(file, wanted < READ_CHUNK ? READ_CHUNK : wanted, &error);
    if (count == -2) { reply_status(work, SLUG_FS_ALLOC); return; }
    if (count < 0) { reply_system(work, error); return; }
    if (count == 0) { reply_status(work, SLUG_FS_END); return; }
  }
  if (wanted > file->end - file->start) wanted = file->end - file->start;
  items[0] = slug_producer_item_i64(SLUG_FS_OK);
  items[1] = slug_producer_item_bytes(file->buffer + file->start, wanted);
  file->start += wanted;
  reply_items(work, items, 2);
}

static void run_read_line(job *work) {
  slug_fs_file *file = work->file;
  /* Bytes after `start` known to hold no newline; kept across refills, which
   * keep the bytes' offsets from `start`. */
  size_t scanned = 0;
  int error = 0;
  for (;;) {
    size_t available = file->end - file->start;
    const uint8_t *text = file->buffer + file->start;
    int64_t count;
    while (scanned < available && text[scanned] != '\n') {
      if (text[scanned] == '\0') { reply_status(work, SLUG_FS_LINE_HAS_NUL); return; }
      if (scanned == MAX_LINE_BYTES) { reply_status(work, SLUG_FS_LINE_TOO_LONG); return; }
      scanned++;
    }
    if (scanned < available) {
      size_t length = scanned;
      if (length > 0 && text[length - 1] == '\r') length--;
      file->start += scanned + 1;
      { reply_text(work, (const char *)text, length); return; }
    }
    count = fill(file, READ_CHUNK, &error);
    if (count == -2) { reply_status(work, SLUG_FS_ALLOC); return; }
    if (count < 0) { reply_system(work, error); return; }
    if (count == 0) {
      /* An unterminated final line is still a line. */
      size_t length = available;
      text = file->buffer + file->start;
      if (length == 0) { reply_status(work, SLUG_FS_END); return; }
      if (text[length - 1] == '\r') length--;
      file->start = file->end;
      { reply_text(work, (const char *)text, length); return; }
    }
  }
}

/* ---- The I/O thread ------------------------------------------------- */

static void run(job *work) {
  switch (work->kind) {
    case JOB_OPEN: run_open(work); break;
    case JOB_READ: run_read(work); break;
    case JOB_READ_LINE: run_read_line(work); break;
    case JOB_WRITE: run_write(work); break;
    case JOB_SYNC: run_sync(work); break;
    case JOB_CLOSE: run_close(work); break;
    case JOB_STAT: run_stat(work); break;
    case JOB_LIST: run_list(work); break;
    case JOB_MAKE_DIRECTORY: run_make_directory(work); break;
    case JOB_RENAME: run_rename(work); break;
    case JOB_REMOVE: run_remove(work); break;
    case JOB_REMOVE_ALL: run_remove_all(work); break;
    case JOB_TEMPORARY_DIRECTORY: run_temporary_directory(work); break;
  }
}

static void finish(job *work) {
  if (work->reply != NULL) slug_producer_release(work->reply);
  if (work->file != NULL) file_unref(work->file);
  free(work->path);
  free(work->other);
  free(work->data);
  free(work);
}

/* The queue's lock and wake-up, and the thread's one-time start. */
#if defined(_WIN32)
static INIT_ONCE start_once = INIT_ONCE_STATIC_INIT;
static SRWLOCK queue_lock = SRWLOCK_INIT;
static CONDITION_VARIABLE queue_ready = CONDITION_VARIABLE_INIT;
static void lock_queue(void) { AcquireSRWLockExclusive(&queue_lock); }
static void unlock_queue(void) { ReleaseSRWLockExclusive(&queue_lock); }
static void wait_queue(void) { SleepConditionVariableSRW(&queue_ready, &queue_lock, INFINITE, 0); }
static void wake_queue(void) { WakeConditionVariable(&queue_ready); }
#else
static pthread_once_t start_once = PTHREAD_ONCE_INIT;
static pthread_mutex_t queue_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t queue_ready = PTHREAD_COND_INITIALIZER;
static void lock_queue(void) { pthread_mutex_lock(&queue_lock); }
static void unlock_queue(void) { pthread_mutex_unlock(&queue_lock); }
static void wait_queue(void) { pthread_cond_wait(&queue_ready, &queue_lock); }
static void wake_queue(void) { pthread_cond_signal(&queue_ready); }
#endif

static void serve(void) {
  for (;;) {
    job *work;
    lock_queue();
    while (queue_head == NULL) wait_queue();
    work = queue_head;
    queue_head = work->next;
    if (queue_head == NULL) queue_tail = NULL;
    unlock_queue();
    run(work);
    finish(work);
  }
}

#if defined(_WIN32)
static DWORD WINAPI serve_thread(LPVOID unused) {
  (void)unused;
  serve();
  return 0;
}

static BOOL CALLBACK start(PINIT_ONCE once, PVOID parameter, PVOID *context) {
  HANDLE thread = CreateThread(NULL, 0, serve_thread, NULL, 0, NULL);
  (void)once;
  (void)parameter;
  (void)context;
  if (thread != NULL) {
    CloseHandle(thread);
    started = true;
  }
  return TRUE;
}

static void start_thread(void) { InitOnceExecuteOnce(&start_once, start, NULL, NULL); }
#else
static void *serve_thread(void *unused) {
  (void)unused;
  serve();
  return NULL;
}

static void start(void) {
  pthread_t thread;
  if (pthread_create(&thread, NULL, serve_thread, NULL) == 0) {
    pthread_detach(thread);
    started = true;
  }
}

static void start_thread(void) { pthread_once(&start_once, start); }
#endif

/* Queues `work`, or, when the I/O thread cannot start, finishes it unsent. */
static int32_t submit(job *work) {
  start_thread();
  if (!started) {
    finish(work);
    return SLUG_FS_NOT_QUEUED;
  }
  work->next = NULL;
  lock_queue();
  if (queue_tail == NULL) {
    queue_head = work;
  } else {
    queue_tail->next = work;
  }
  queue_tail = work;
  wake_queue();
  unlock_queue();
  return SLUG_FS_OK;
}

/* A job for `file` (which it holds a reference to) and `reply`, or NULL with
 * the reply released. */
static job *new_job(job_kind kind, slug_fs_file *file, slug_producer *reply) {
  job *work = calloc(1, sizeof *work);
  if (work == NULL) {
    if (reply != NULL) slug_producer_release(reply);
    return NULL;
  }
  work->kind = kind;
  work->reply = reply;
  if (file != NULL) {
    atomic_fetch_add(&file->references, 1);
    work->file = file;
  }
  return work;
}

static bool copy_text(char **target, const char *text) {
  size_t length = strlen(text);
  *target = malloc(length + 1);
  if (*target == NULL) return false;
  memcpy(*target, text, length + 1);
  return true;
}

static int32_t submit_path(job_kind kind, const char *path, int64_t number,
                           slug_producer *reply) {
  job *work = new_job(kind, NULL, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  work->number = number;
  if (!copy_text(&work->path, path)) {
    finish(work);
    return SLUG_FS_NOT_QUEUED;
  }
  return submit(work);
}

/* ---- Entry points --------------------------------------------------- */

int32_t slug_fs_open(const char *path, int64_t mode, slug_producer *reply, slug_fs_file **file) {
  slug_fs_file *opened = calloc(1, sizeof *opened);
  job *work;
  *file = NULL;
  if (opened == NULL) {
    slug_producer_release(reply);
    return SLUG_FS_NOT_QUEUED;
  }
  atomic_init(&opened->references, 1);
  opened->handle = NO_HANDLE;
  opened->mode = mode == SLUG_FS_WRITE || mode == SLUG_FS_APPEND ? mode : SLUG_FS_READ;
  work = new_job(JOB_OPEN, opened, reply);
  if (work == NULL) {
    file_unref(opened);
    return SLUG_FS_NOT_QUEUED;
  }
  work->number = mode;
  if (!copy_text(&work->path, path)) {
    finish(work);
    file_unref(opened);
    return SLUG_FS_NOT_QUEUED;
  }
  if (submit(work) != SLUG_FS_OK) {
    file_unref(opened);
    return SLUG_FS_NOT_QUEUED;
  }
  *file = opened;
  return SLUG_FS_OK;
}

int32_t slug_fs_read(slug_fs_file *file, int64_t max_bytes, slug_producer *reply) {
  job *work = new_job(JOB_READ, file, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  if (max_bytes < 1) max_bytes = 1;
  if ((uint64_t)max_bytes > MAX_READ_BYTES) max_bytes = (int64_t)MAX_READ_BYTES;
  work->number = max_bytes;
  return submit(work);
}

int32_t slug_fs_read_line(slug_fs_file *file, slug_producer *reply) {
  job *work = new_job(JOB_READ_LINE, file, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  return submit(work);
}

int32_t slug_fs_write(slug_fs_file *file, const uint8_t *data, size_t length,
                      slug_producer *reply) {
  job *work = new_job(JOB_WRITE, file, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  if (length > 0) {
    work->data = malloc(length);
    if (work->data == NULL) {
      finish(work);
      return SLUG_FS_NOT_QUEUED;
    }
    memcpy(work->data, data, length);
  }
  work->length = length;
  return submit(work);
}

int32_t slug_fs_write_text(slug_fs_file *file, const char *text, size_t length,
                           slug_producer *reply) {
  return slug_fs_write(file, (const uint8_t *)text, length, reply);
}

int32_t slug_fs_sync(slug_fs_file *file, slug_producer *reply) {
  job *work = new_job(JOB_SYNC, file, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  return submit(work);
}

void slug_fs_release(slug_fs_file *file) {
  job *work = new_job(JOB_CLOSE, file, NULL);
  /* Without memory for the job the handle leaks: closing it here could pull
   * it from under work the I/O thread is doing, and a reused number would
   * then reach another file. */
  if (work != NULL) submit(work);
  file_unref(file);
}

int32_t slug_fs_stat(const char *path, slug_producer *reply) {
  return submit_path(JOB_STAT, path, 0, reply);
}

int32_t slug_fs_list(const char *path, slug_producer *reply) {
  return submit_path(JOB_LIST, path, 0, reply);
}

int32_t slug_fs_make_directory(const char *path, int64_t parents, slug_producer *reply) {
  return submit_path(JOB_MAKE_DIRECTORY, path, parents, reply);
}

int32_t slug_fs_rename(const char *from, const char *to, slug_producer *reply) {
  job *work = new_job(JOB_RENAME, NULL, reply);
  if (work == NULL) return SLUG_FS_NOT_QUEUED;
  if (!copy_text(&work->path, from) || !copy_text(&work->other, to)) {
    finish(work);
    return SLUG_FS_NOT_QUEUED;
  }
  return submit(work);
}

int32_t slug_fs_remove(const char *path, slug_producer *reply) {
  return submit_path(JOB_REMOVE, path, 0, reply);
}

int32_t slug_fs_remove_all(const char *path, slug_producer *reply) {
  return submit_path(JOB_REMOVE_ALL, path, 0, reply);
}

int32_t slug_fs_temporary_directory(const char *prefix, slug_producer *reply) {
  return submit_path(JOB_TEMPORARY_DIRECTORY, prefix, 0, reply);
}

const char *slug_fs_separator(void) {
#if defined(_WIN32)
  return "\\";
#else
  return "/";
#endif
}

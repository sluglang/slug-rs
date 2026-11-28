/* Deterministic backend tests: no dependence on FSEvents batch timing or the
 * host exhausting its OS event queue. Include the private implementation so
 * overflow and native cancellation state can be exercised directly. */
#include "watch.c"
#include <assert.h>
#include <stdio.h>

typedef struct {
  slug_producer producer;
  atomic_bool closed;
  atomic_bool occupied;
  bool full_reply;
  bool invalid_reply;
  atomic_int releases;
  int full_notifications;
  int64_t status;
  bool rescan;
  size_t count;
  char path[64];
} mailbox;

static int32_t send_bool(slug_producer *producer, bool value) {
  mailbox *box = (mailbox *)producer;
  assert(value);
  if (box->closed) return SLUG_PRODUCER_CLOSED;
  if (box->occupied) { box->full_notifications++; return SLUG_PRODUCER_FULL; }
  box->occupied = true;
  return SLUG_PRODUCER_SENT;
}

static int32_t send_list(slug_producer *producer, const slug_producer_item *items, size_t count) {
  mailbox *box = (mailbox *)producer;
  if (box->closed) return SLUG_PRODUCER_CLOSED;
  if (box->full_reply) return SLUG_PRODUCER_FULL;
  if (box->invalid_reply && count > 2) return SLUG_PRODUCER_INVALID;
  box->count = count;
  box->status = items[0].integer;
  box->rescan = count > 1 && items[1].boolean;
  if (count > 2 && items[2].kind == SLUG_PRODUCER_ITEM_TEXT) {
    size_t length = items[2].length;
    if (length >= sizeof(box->path)) length = sizeof(box->path) - 1;
    memcpy(box->path, items[2].data, length);
    box->path[length] = 0;
  }
  return SLUG_PRODUCER_SENT;
}

static bool is_closed(slug_producer *producer) { return ((mailbox *)producer)->closed; }
static void close_box(slug_producer *producer) { ((mailbox *)producer)->closed = true; }
static void release_box(slug_producer *producer) { ((mailbox *)producer)->releases++; }

static const slug_producer_api api = {
  .version = SLUG_PRODUCER_API_VERSION,
  .send_bool = send_bool,
  .is_closed = is_closed,
  .close = close_box,
  .release = release_box,
  .send_list = send_list,
};

static mailbox box(void) { return (mailbox){.producer = {&api}}; }

static slug_fs_watch *make_watch(mailbox *ready, bool recursive) {
  slug_fs_watch *watch = NULL;
  assert(slug_fs_watch_create(".", recursive, &ready->producer, &watch) == SLUG_FS_OK);
  assert(watch != NULL);
  return watch;
}

static mailbox drain(slug_fs_watch *watch) {
  mailbox reply = box();
  assert(slug_fs_watch_drain(watch, &reply.producer) == 0);
  assert(reply.closed && reply.releases == 1);
  return reply;
}

static void overflow_survives_full_channel_and_silence(void) {
  mailbox ready = box();
  slug_fs_watch *watch = make_watch(&ready, true);
  char name[32];
  for (size_t i = 0; i <= WATCH_PATHS; i++) {
    snprintf(name, sizeof(name), "page-%zu", i);
    record_path(watch, name);
  }
  assert(ready.occupied && ready.full_notifications > 0);
  assert(watch->count == 0 && watch->bytes == 0 && watch->rescan);
  /* No later event is required to make overflow visible. A failed drain
   * cannot acknowledge it either. */
  mailbox full = box();
  full.full_reply = true;
  slug_fs_watch_drain(watch, &full.producer);
  assert(watch->rescan);
  mailbox result = drain(watch);
  assert(result.status == SLUG_FS_OK && result.rescan && result.count == 2);
  result = drain(watch);
  assert(result.status == SLUG_FS_OK && !result.rescan && result.count == 2);
  slug_fs_watch_release(watch);
  assert(ready.closed && ready.releases == 1);
}

static void byte_limit_and_unrepresentable_path_require_rescan(void) {
  mailbox ready = box();
  slug_fs_watch *watch = make_watch(&ready, true);
  char *large = malloc(WATCH_BYTES + 1);
  assert(large != NULL);
  memset(large, 'a', WATCH_BYTES);
  large[WATCH_BYTES] = 0;
  record_path(watch, large);
  free(large);
  assert(drain(watch).rescan);
  record_path(watch, "invalid-utf8");
  mailbox invalid = box();
  invalid.invalid_reply = true;
  slug_fs_watch_drain(watch, &invalid.producer);
  assert(invalid.rescan && invalid.count == 2);
  assert(!watch->rescan && watch->count == 0);
  slug_fs_watch_release(watch);
}

static void exclusions_and_nonrecursive_scope_use_components(void) {
  mailbox ready = box();
  slug_fs_watch *watch = make_watch(&ready, false);
  assert(slug_fs_watch_exclude(watch, "dist") == SLUG_FS_OK);
  record_path(watch, "dist");
  record_path(watch, "dist/page.html");
  record_path(watch, "nested/page.md");
  assert(!ready.occupied && drain(watch).count == 2);
  record_path(watch, "distance.md");
  record_path(watch, "distance.md");
  mailbox result = drain(watch);
  assert(result.count == 3 && strcmp(result.path, "distance.md") == 0);
  /* A stale wakeup after drain must still work as a wakeup for later data. */
  record_path(watch, "later.md");
  assert(ready.occupied);
  ready.occupied = false;
  result = drain(watch);
  assert(result.count == 3 && strcmp(result.path, "later.md") == 0);
  record_path(watch, "after-drain.md");
  assert(ready.occupied);
  slug_fs_watch_stop(watch);
  slug_fs_watch_stop(watch);
  assert(drain(watch).status == SLUG_FS_END);
  slug_fs_watch_release(watch);
  assert(ready.releases == 1);
}

static void os_loss_and_terminal_errors_are_retained(void) {
  mailbox ready = box();
  slug_fs_watch *watch = make_watch(&ready, true);
  watch->root = strdup("/watched");
  assert(watch->root != NULL);
  char *paths[] = {"/watched/a"};
  FSEventStreamEventId ids[] = {1};
  FSEventStreamEventFlags flags[] = {kFSEventStreamEventFlagMustScanSubDirs};
  events(NULL, watch, 1, paths, flags, ids);
  assert(drain(watch).rescan);
  flags[0] = kFSEventStreamEventFlagRootChanged;
  events(NULL, watch, 1, paths, flags, ids);
  mailbox result = drain(watch);
  assert(result.status == SLUG_FS_SYSTEM && strcmp(result.path, "watch_lost") == 0);
  slug_fs_watch_stop(watch);
  assert(drain(watch).status == SLUG_FS_SYSTEM);
  slug_fs_watch_release(watch);
}

static void idle_worker_stops_on_close_or_receiver_revocation(void) {
  for (int revoke = 0; revoke < 2; revoke++) {
    mailbox ready = box();
    mailbox reply = box();
    slug_fs_watch *watch = make_watch(&ready, true);
    assert(slug_fs_watch_start(watch, &reply.producer) == 0);
    struct timespec pause = {0, 1000000};
    int tries = 0;
    while (!reply.closed && tries++ < 5000) nanosleep(&pause, NULL);
    assert(reply.closed && reply.status == SLUG_FS_OK);
    if (revoke) close_box(&ready.producer);
    else slug_fs_watch_stop(watch);
    tries = 0;
    while (atomic_load(&watch->references) != 1 && tries++ < 5000) nanosleep(&pause, NULL);
    assert(atomic_load(&watch->references) == 1);
    slug_fs_watch_release(watch);
    assert(ready.closed && ready.releases == 1);
  }
}

int main(void) {
  overflow_survives_full_channel_and_silence();
  byte_limit_and_unrepresentable_path_require_rescan();
  exclusions_and_nonrecursive_scope_use_components();
  os_loss_and_terminal_errors_are_retained();
  idle_worker_stops_on_close_or_receiver_revocation();
  puts("watch invariants passed");
  return 0;
}

/* Compositor-side regression probe, never linked into Ruby Reader.
 * Generate wlr-toplevel.h/.c with wayland-scanner, then link libwayland-client.
 * Lists actual toplevels; --close only accepts our uniquely titled smoke windows.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>
#include "wlr-toplevel.h"

struct entry { struct zwlr_foreign_toplevel_handle_v1 *handle; char *title; int closed; };
static struct entry entries[256];
static size_t count;
static struct zwlr_foreign_toplevel_manager_v1 *manager;
static void title(void *data, struct zwlr_foreign_toplevel_handle_v1 *h, const char *s) {
    (void)h; struct entry *e = data; free(e->title); e->title = strdup(s);
}
static void app_id(void *d, struct zwlr_foreign_toplevel_handle_v1 *h, const char *s) {(void)d;(void)h;(void)s;}
static void output(void *d, struct zwlr_foreign_toplevel_handle_v1 *h, struct wl_output *o) {(void)d;(void)h;(void)o;}
static void state(void *d, struct zwlr_foreign_toplevel_handle_v1 *h, struct wl_array *s) {(void)d;(void)h;(void)s;}
static void done(void *d, struct zwlr_foreign_toplevel_handle_v1 *h) {(void)d;(void)h;}
static void closed(void *d, struct zwlr_foreign_toplevel_handle_v1 *h) {(void)h; ((struct entry *)d)->closed = 1;}
static void parent(void *d, struct zwlr_foreign_toplevel_handle_v1 *h, struct zwlr_foreign_toplevel_handle_v1 *p) {(void)d;(void)h;(void)p;}
static const struct zwlr_foreign_toplevel_handle_v1_listener listener = {
    .title=title, .app_id=app_id, .output_enter=output, .output_leave=output,
    .state=state, .done=done, .closed=closed, .parent=parent
};
static void toplevel(void *d, struct zwlr_foreign_toplevel_manager_v1 *m, struct zwlr_foreign_toplevel_handle_v1 *h) {
    (void)d;(void)m;
    if (count == 256) return;
    struct entry *e = &entries[count++]; e->handle = h;
    zwlr_foreign_toplevel_handle_v1_add_listener(h, &listener, e);
}
static void finished(void *d, struct zwlr_foreign_toplevel_manager_v1 *m) {(void)d;(void)m;}
static const struct zwlr_foreign_toplevel_manager_v1_listener manager_listener = {toplevel, finished};
static void global(void *d, struct wl_registry *r, uint32_t name, const char *interface, uint32_t version) {
    (void)d;
    if (!strcmp(interface, zwlr_foreign_toplevel_manager_v1_interface.name)) {
        manager = wl_registry_bind(r, name, &zwlr_foreign_toplevel_manager_v1_interface, version < 3 ? version : 3);
        zwlr_foreign_toplevel_manager_v1_add_listener(manager, &manager_listener, NULL);
    }
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) {(void)d;(void)r;(void)n;}
static const struct wl_registry_listener registry_listener = {global, global_remove};
int main(int argc, char **argv) {
    const char *target = NULL;
    if (argc == 3 && !strcmp(argv[1], "--close") && !strncmp(argv[2], "Ruby Reader test ", 17)) target = argv[2];
    else if (argc != 1) { fputs("Usage: probe [--close 'Ruby Reader test PID']\n", stderr); return 2; }
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 3;
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    for (int i = 0; i < 3; ++i) if (wl_display_roundtrip(display) < 0) return 4;
    if (!manager) return 5;
    int found = 0;
    for (size_t i = 0; i < count; ++i) {
        struct entry *e = &entries[i];
        if (!e->title || e->closed) continue;
        if (!target) puts(e->title);
        else if (!strcmp(target, e->title)) { zwlr_foreign_toplevel_handle_v1_close(e->handle); found++; }
    }
    wl_display_roundtrip(display);
    wl_display_disconnect(display);
    for (size_t i = 0; i < count; ++i) free(entries[i].title);
    return target && found != 1 ? 6 : 0;
}

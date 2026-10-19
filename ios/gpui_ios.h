#pragma once
#include <stdint.h>
#include <stdbool.h>
typedef struct { double x, y, width, height; } GPUIInputRect;
void gpui_ios_frame(uint64_t id);
void gpui_ios_resize(uint64_t id, float width, float height, float scale, bool dark);
void gpui_ios_insets(uint64_t id, float top, float right, float bottom, float left, float keyboard);
void gpui_ios_touch(uint64_t id, uint64_t touch, int32_t phase, float x, float y, float force);
void gpui_ios_key(uint64_t id, const char *key, uint32_t modifiers);
void gpui_ios_lifecycle(int32_t phase);
void gpui_ios_memory_warning(void);
void gpui_ios_shutdown(void);
char *gpui_ios_text_snapshot(uint64_t id);
void gpui_ios_string_free(char *text);
void gpui_ios_replace_text(uint64_t id, int64_t start, int64_t end, const char *text, bool marked, int64_t selected_start, int64_t selected_end);
void gpui_ios_set_selection(uint64_t id, uint64_t start, uint64_t end);
void gpui_ios_delete_backward(uint64_t id);
void gpui_ios_unmark(uint64_t id);
GPUIInputRect gpui_ios_text_rect(uint64_t id, uint64_t start, uint64_t end);
int64_t gpui_ios_text_index(uint64_t id, float x, float y);
void gpui_ios_accessibility_enabled(uint64_t id, bool enabled);
void gpui_ios_accessibility_action(uint64_t id, uint64_t node, bool focus);

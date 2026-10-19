use crate::{backend::*, unicode};
use gpui::*;
use serde_json::json;
use std::{
    collections::BTreeSet,
    ffi::{CStr, CString},
    panic::{AssertUnwindSafe, catch_unwind},
};

fn guarded<T: Default>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("gpui_ios: rejected a reentrant or panicking native callback");
        T::default()
    })
}

pub(crate) fn take_host_string(pointer: *mut i8) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: Swift's host ABI returns a strdup allocation; it is freed by the matching host function.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { gpui_ios_host_free_string(pointer) };
    Some(text)
}

fn c_string(text: String) -> *mut i8 {
    CString::new(text)
        .map(CString::into_raw)
        .unwrap_or_default()
}
fn document(handler: &mut PlatformInputHandler) -> String {
    let Some(length) = handler.text_length_utf16() else {
        return String::new();
    };
    handler
        .text_for_range(0..length, &mut None)
        .unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_string_free(text: *mut i8) {
    if !text.is_null() {
        unsafe { drop(CString::from_raw(text)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_frame(id: u64) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        if !state.visible.get() {
            return;
        }
        let callback = state.frame.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(RequestFrameOptions::default());
            *state.frame.borrow_mut() = Some(callback);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_resize(id: u64, width: f32, height: f32, scale: f32, dark: bool) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        if !(width > 0. && height > 0. && scale > 0.) {
            return;
        }
        let dimensions = size(px(width), px(height));
        let bounds_changed = state
            .bounds
            .replace(Bounds::new(Point::default(), dimensions))
            .size
            != dimensions;
        let scale_changed = state.scale.replace(scale) != scale;
        let changed = bounds_changed || scale_changed;
        if changed {
            state.renderer.borrow_mut().update_drawable_size(size(
                DevicePixels((width * scale) as i32),
                DevicePixels((height * scale) as i32),
            ));
            if let Some(callback) = state.resized.borrow_mut().as_mut() {
                callback(dimensions, scale);
            }
        }
        let appearance = if dark {
            WindowAppearance::Dark
        } else {
            WindowAppearance::Light
        };
        if state.appearance.replace(appearance) != appearance {
            if let Some(callback) = state.appearance_changed.borrow_mut().as_mut() {
                callback();
            }
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_insets(
    id: u64,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
    keyboard: f32,
) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        let insets = WindowInsets {
            safe_area: Edges {
                top: px(top),
                right: px(right),
                bottom: px(bottom),
                left: px(left),
            },
            ime: Edges {
                bottom: px(keyboard),
                ..Default::default()
            },
        };
        if *state.insets.borrow() == insets {
            return;
        }
        *state.insets.borrow_mut() = insets.clone();
        if let Some(callback) = state.inset_changed.borrow_mut().as_mut() {
            callback(insets);
        }
        if let Some(callback) = state.viewport_changed.borrow_mut().as_mut() {
            callback();
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_touch(id: u64, touch: u64, phase: i32, x: f32, y: f32, force: f32) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        let phase = match phase {
            0 => TouchPhase::Started,
            1 => TouchPhase::Moved,
            2 => TouchPhase::Ended,
            _ => TouchPhase::Cancelled,
        };
        let event = PlatformInput::Touch(TouchEvent {
            id: TouchId(touch),
            phase,
            position: point(px(x), px(y)),
            predicted_position: None,
            force: (force >= 0.).then_some(force.clamp(0., 1.)),
        });
        let callback = state.input.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(event);
            *state.input.borrow_mut() = Some(callback);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_key(id: u64, key: *const i8, modifiers: u32) {
    guarded(|| {
        if key.is_null() {
            return;
        }
        let key = unsafe { CStr::from_ptr(key) }
            .to_string_lossy()
            .into_owned();
        let Some(state) = window(id) else {
            return;
        };
        let event = PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers {
                    shift: modifiers & 1 != 0,
                    control: modifiers & 2 != 0,
                    alt: modifiers & 4 != 0,
                    platform: modifiers & 8 != 0,
                    function: false,
                },
                key,
                key_char: None,
            },
            is_held: false,
            prefer_character_input: false,
        });
        let callback = state.input.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(event);
            *state.input.borrow_mut() = Some(callback);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_lifecycle(phase: i32) {
    guarded(|| {
        let phase = match phase {
            0 => AppLifecyclePhase::Active,
            1 => AppLifecyclePhase::Inactive,
            2 => AppLifecyclePhase::Background,
            _ => AppLifecyclePhase::Foreground,
        };
        let windows =
            WINDOWS.with(|windows| windows.borrow().values().cloned().collect::<Vec<_>>());
        for state in windows {
            let active = phase == AppLifecyclePhase::Active;
            let visible = phase != AppLifecyclePhase::Background;
            if state.active.replace(active) != active {
                if let Some(callback) = state.active_changed.borrow_mut().as_mut() {
                    callback(active);
                }
            }
            if state.visible.replace(visible) != visible {
                if let Some(callback) = state.visibility_changed.borrow_mut().as_mut() {
                    callback(if visible {
                        WindowVisibility::Visible
                    } else {
                        WindowVisibility::Hidden
                    });
                }
            }
        }
        PLATFORM.with(|platform| {
            if let Some(platform) = platform.borrow().as_ref() {
                if let Some(callback) = platform.lifecycle.borrow_mut().as_mut() {
                    callback(phase);
                }
            }
        });
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_memory_warning() {
    guarded(|| {
        PLATFORM.with(|platform| {
            if let Some(platform) = platform.borrow().as_ref() {
                if let Some(callback) = platform.memory_warning.borrow_mut().as_mut() {
                    callback();
                }
            }
        })
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_shutdown() {
    guarded(crate::backend::shutdown);
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_text_snapshot(id: u64) -> *mut i8 {
    guarded(|| {
        with_input(id, |handler| {
            let text = document(handler);
            let selection = handler
                .selected_text_range(false)
                .map(|s| vec![s.range.start, s.range.end]);
            let marked = handler.marked_text_range().map(|s| vec![s.start, s.end]);
            c_string(json!({ "text": text, "selection": selection, "marked": marked }).to_string())
        })
        .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_replace_text(
    id: u64,
    start: i64,
    end: i64,
    text: *const i8,
    marked: bool,
    selected_start: i64,
    selected_end: i64,
) {
    guarded(|| {
        if text.is_null() {
            return;
        }
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        with_input(id, |handler| {
            let range = (start >= 0 && end >= start).then(|| {
                unicode::normalized_range(&document(handler), start as usize..end as usize)
            });
            if marked {
                let selection =
                    (selected_start >= 0 && selected_end >= selected_start).then(|| {
                        unicode::normalized_range(
                            &text,
                            selected_start as usize..selected_end as usize,
                        )
                    });
                handler.replace_and_mark_text_in_range(range, &text, selection);
            } else {
                handler.replace_text_in_range(range, &text);
            }
        });
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_set_selection(id: u64, start: u64, end: u64) {
    guarded(|| {
        with_input(id, |handler| {
            let range = unicode::normalized_range(
                &document(handler),
                start.min(end) as usize..start.max(end) as usize,
            );
            handler.set_selected_text_range(range);
        });
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_unmark(id: u64) {
    guarded(|| {
        with_input(id, |handler| handler.unmark_text());
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_delete_backward(id: u64) {
    guarded(|| {
        with_input(id, |handler| {
            let Some(selection) = handler.selected_text_range(false) else {
                return;
            };
            let text = document(handler);
            let range = if selection.range.is_empty() {
                unicode::previous_grapheme(&text, selection.range.start)
            } else {
                selection.range
            };
            if !range.is_empty() {
                handler.replace_text_in_range(Some(range), "");
            }
        });
    });
}

#[repr(C)]
#[derive(Default)]
pub struct InputRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_text_rect(id: u64, start: u64, end: u64) -> InputRect {
    guarded(|| {
        with_input(id, |handler| {
            handler.bounds_for_range(start as usize..end as usize)
        })
        .flatten()
        .map(|b| InputRect {
            x: b.origin.x.as_f32() as f64,
            y: b.origin.y.as_f32() as f64,
            width: b.size.width.as_f32() as f64,
            height: b.size.height.as_f32() as f64,
        })
        .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_text_index(id: u64, x: f32, y: f32) -> i64 {
    guarded(|| {
        with_input(id, |handler| {
            handler.character_index_for_point(point(px(x), px(y)))
        })
        .flatten()
        .map(|i| i as i64)
        .unwrap_or(-1)
    })
}

pub(crate) fn accessibility_update(state: &WindowState, update: accesskit::TreeUpdate) {
    if let Some(tree) = update.tree {
        state.a11y_root.set(Some(tree.root.0));
    }
    let mut nodes = state.nodes.borrow_mut();
    for (id, node) in update.nodes {
        nodes.insert(id.0, node);
    }
    let mut reachable = BTreeSet::new();
    let mut reading_order = Vec::new();
    let mut stack = state.a11y_root.get().into_iter().collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        reading_order.push(id);
        if let Some(node) = nodes.get(&id) {
            stack.extend(node.children().iter().rev().map(|id| id.0));
        }
    }
    nodes.retain(|id, _| reachable.contains(id));
    let scale = f64::from(state.scale.get());
    let exported = reading_order.iter().filter_map(|id| {
        let node = nodes.get(id)?;
        let bounds = node.bounds()?;
        Some(json!({ "id": id.to_string(), "role": format!("{:?}", node.role()), "label": node.label(), "value": node.value(), "disabled": node.is_disabled(), "bounds": [bounds.x0 / scale, bounds.y0 / scale, (bounds.x1 - bounds.x0) / scale, (bounds.y1 - bounds.y0) / scale], "click": node.supports_action(accesskit::Action::Click), "focus": node.supports_action(accesskit::Action::Focus) }))
    }).collect::<Vec<_>>();
    drop(nodes);
    if let Ok(json) = CString::new(json!(exported).to_string()) {
        unsafe { gpui_ios_host_accessibility(state.view.as_ptr(), json.as_ptr()) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_accessibility_enabled(id: u64, enabled: bool) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        if state.a11y_active.replace(enabled) == enabled {
            return;
        }
        let callbacks = state.accessibility.borrow_mut().take();
        if let Some(callbacks) = callbacks {
            if enabled {
                if let Some(update) = (callbacks.activation)() {
                    accessibility_update(&state, update);
                }
            } else {
                (callbacks.deactivation)();
            }
            *state.accessibility.borrow_mut() = Some(callbacks);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_accessibility_action(id: u64, node: u64, focus: bool) {
    guarded(|| {
        let Some(state) = window(id) else {
            return;
        };
        let callbacks = state.accessibility.borrow_mut().take();
        if let Some(callbacks) = callbacks {
            (callbacks.action)(accesskit::ActionRequest {
                action: if focus {
                    accesskit::Action::Focus
                } else {
                    accesskit::Action::Click
                },
                target_tree: accesskit::TreeId::ROOT,
                target_node: accesskit::NodeId(node),
                data: None,
            });
            *state.accessibility.borrow_mut() = Some(callbacks);
        }
    });
}

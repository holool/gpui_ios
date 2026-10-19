#[cfg(target_os = "ios")]
mod app {
    use gpui::{prelude::*, *};
    use std::ops::Range;

    struct Demo {
        text: String,
        selection: Range<usize>,
        marked: Option<Range<usize>>,
        focus: FocusHandle,
        line: Option<ShapedLine>,
        origin: Point<Pixels>,
        taps: usize,
    }
    impl Demo {
        fn new(cx: &mut Context<Self>) -> Self {
            let text = "مرحباً بالعالم — GPUI Fast 👋".to_owned();
            let end = text.encode_utf16().count();
            Self {
                text,
                selection: end..end,
                marked: None,
                focus: cx.focus_handle(),
                line: None,
                origin: Point::default(),
                taps: 0,
            }
        }
        fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> Range<usize> {
            let range = range
                .or(self.marked.clone())
                .unwrap_or(self.selection.clone());
            let bytes = gpui_ios::unicode::utf16_range(&self.text, range.clone());
            self.text.replace_range(bytes, text);
            let end = range.start + text.encode_utf16().count();
            self.selection = end..end;
            range.start..end
        }
    }
    impl EntityInputHandler for Demo {
        fn text_for_range(
            &mut self,
            range: Range<usize>,
            adjusted: &mut Option<Range<usize>>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<String> {
            *adjusted = Some(gpui_ios::unicode::normalized_range(
                &self.text,
                range.clone(),
            ));
            Some(self.text[gpui_ios::unicode::utf16_range(&self.text, range)].to_owned())
        }
        fn selected_text_range(
            &mut self,
            _: bool,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<UTF16Selection> {
            Some(UTF16Selection {
                range: self.selection.clone(),
                reversed: false,
            })
        }
        fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
            self.marked.clone()
        }
        fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
            self.marked = None;
            cx.notify();
        }
        fn replace_text_in_range(
            &mut self,
            range: Option<Range<usize>>,
            text: &str,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.replace(range, text);
            self.marked = None;
            cx.notify();
        }
        fn replace_and_mark_text_in_range(
            &mut self,
            range: Option<Range<usize>>,
            text: &str,
            selected: Option<Range<usize>>,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let marked = self.replace(range, text);
            if let Some(selected) = selected {
                self.selection = marked.start + selected.start..marked.start + selected.end;
            }
            self.marked = (!text.is_empty()).then_some(marked);
            cx.notify();
        }
        fn bounds_for_range(
            &mut self,
            range: Range<usize>,
            bounds: Bounds<Pixels>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<Bounds<Pixels>> {
            let line = self.line.as_ref()?;
            let bytes = gpui_ios::unicode::utf16_range(&self.text, range);
            let x = line.x_for_index(bytes.start);
            let end_x = line.x_for_index(bytes.end);
            Some(Bounds::new(
                point(bounds.origin.x + x.min(end_x), bounds.origin.y),
                size((end_x - x).abs(), bounds.size.height),
            ))
        }
        fn character_index_for_point(
            &mut self,
            position: Point<Pixels>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<usize> {
            let index = self
                .line
                .as_ref()?
                .closest_index_for_x(position.x - self.origin.x);
            Some(
                self.text[..index.min(self.text.len())]
                    .encode_utf16()
                    .count(),
            )
        }
        fn set_selected_text_range(
            &mut self,
            range: Range<usize>,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.selection = range;
            cx.notify();
        }
        fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
            Some(self.text.encode_utf16().count())
        }
    }
    impl Render for Demo {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let visible = window.fully_visible_bounds();
            let bottom_inset = window.viewport_size().height - visible.bottom();
            let entity = cx.entity();
            let text: SharedString = self.text.clone().into();
            let focus = self.focus.clone();
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .px_5()
                .pt(visible.origin.y + px(20.))
                .pb(bottom_inset + px(20.))
                .bg(rgb(0x101820))
                .text_color(rgb(0xf0f4f8))
                .child(div().text_2xl().child("GPUI Fast · iOS"))
                .child(div().child("Tap the field to type Arabic, emoji, or composing IME text."))
                .child(
                    div()
                        .id("input")
                        .track_focus(&self.focus)
                        .role(Role::TextInput)
                        .aria_label("Native text input")
                        .aria_value(self.text.clone())
                        .border_1()
                        .border_color(rgb(0x77ccff))
                        .rounded_lg()
                        .p_3()
                        .h(px(64.))
                        .w_full()
                        .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
                        .child(
                            canvas(
                                move |bounds, window, _| {
                                    let style = window.text_style();
                                    let run = TextRun {
                                        len: text.len(),
                                        font: style.font(),
                                        color: style.color,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    (
                                        bounds,
                                        window.text_system().shape_line(
                                            text,
                                            px(21.),
                                            &[run],
                                            None,
                                        ),
                                    )
                                },
                                move |bounds, (_, line), window, cx| {
                                    entity.update(cx, |input, _| {
                                        input.line = Some(line.clone());
                                        input.origin = bounds.origin;
                                    });
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, entity.clone()),
                                        cx,
                                    );
                                    if let Err(error) = line.paint(
                                        bounds.origin,
                                        px(30.),
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    ) {
                                        log_error(error);
                                    }
                                },
                            )
                            .size_full(),
                        ),
                )
                .child(
                    div()
                        .id("counter")
                        .role(Role::Button)
                        .aria_label("Touch counter")
                        .aria_value(self.taps.to_string())
                        .p_4()
                        .rounded_lg()
                        .bg(rgb(0x245b78))
                        .child(format!("Touch counter: {}", self.taps))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.taps += 1;
                            cx.notify();
                        })),
                )
                .child(
                    div().child(
                        "UIKit lifecycle · portable touch gestures · shared Metal + CoreText",
                    ),
                )
        }
    }
    fn log_error(error: impl std::fmt::Display) {
        eprintln!("paint failed: {error}");
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn gpui_ios_example_launch() {
        gpui_ios::run(|cx| {
            cx.open_window(WindowOptions::default(), |_, cx| cx.new(Demo::new))
                .expect("create native example window");
        });
    }
}

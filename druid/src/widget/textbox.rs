// Copyright 2018 The Druid Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A textbox widget.

use std::time::Duration;

use crate::kurbo::Vec2;
use crate::piet::PietText;
use crate::text::{
    format::Formatter, BasicTextInput, EditAction, EditableText, Editor, Selection, TextInput,
    TextLayout, TextStorage,
};
use crate::widget::prelude::*;
use crate::{
    theme, Affine, Color, Cursor, Data, FontDescriptor, HotKey, Insets, KbKey, KeyOrValue, Point,
    Selector, SysMods, TimerToken,
};

const MAC_OR_LINUX: bool = cfg!(any(target_os = "macos", target_os = "linux"));

const BORDER_WIDTH: f64 = 1.;
const TEXT_INSETS: Insets = Insets::new(4.0, 2.0, 0.0, 2.0);

const CURSOR_BLINK_DURATION: Duration = Duration::from_millis(500);

const COMPLETE_EDITING: Selector = Selector::new("druid.builtin.textbox-complete-editing");
const CANCEL_EDITING: Selector = Selector::new("druid.builtin.textbox-cancel-editing");

/// A widget that allows user text input.
#[derive(Debug, Clone)]
pub struct TextBox<T> {
    placeholder: TextLayout<String>,
    editor: Editor<T>,
    // this can be Box<dyn TextInput> in the future
    input_handler: BasicTextInput,
    hscroll_offset: f64,
    // in cases like SelectAll, we don't adjust the viewport after an event.
    suppress_adjust_hscroll: bool,
    cursor_timer: TimerToken,
    cursor_on: bool,
    multiline: bool,
    /// true if a click event caused us to gain focus.
    ///
    /// On macOS, if focus happens via click then we set the selection based
    /// on the click position; if focus happens automatically (e.g. on tab)
    /// then we select our entire contents.
    was_focused_from_click: bool,
}

/// A `TextBox` that uses a [`Formatter`] to handle formatting and validation
/// of its data.
///
/// [`Formatter`]: crate::text::Formatter;
pub struct ValueTextBox<T> {
    inner: TextBox<String>,
    formatter: Box<dyn Formatter<T>>,
    is_editing: bool,
    force_selection: Option<Selection>,
    old_buffer: String,
    buffer: String,
}

impl TextBox<()> {
    /// Perform an `EditAction`. The payload *must* be an `EditAction`.
    pub const PERFORM_EDIT: Selector<EditAction> =
        Selector::new("druid-builtin.textbox.perform-edit");
}

impl<T> TextBox<T> {
    /// Create a new TextBox widget
    pub fn new() -> Self {
        let mut placeholder = TextLayout::from_text("");
        placeholder.set_text_color(theme::PLACEHOLDER_COLOR);
        Self {
            editor: Editor::new(),
            input_handler: BasicTextInput::default(),
            hscroll_offset: 0.,
            suppress_adjust_hscroll: false,
            cursor_timer: TimerToken::INVALID,
            cursor_on: false,
            placeholder,
            multiline: false,
            was_focused_from_click: false,
        }
    }

    /// Create a new multi-line `TextBox`.
    pub fn multiline() -> Self {
        let mut this = TextBox::new();
        this.editor.set_multiline(true);
        this.multiline = true;
        this
    }

    /// Builder-style method to set the `TextBox`'s placeholder text.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder.set_text(placeholder.into());
        self
    }

    /// Builder-style method for setting the text size.
    ///
    /// The argument can be either an `f64` or a [`Key<f64>`].
    ///
    /// [`Key<f64>`]: ../struct.Key.html
    pub fn with_text_size(mut self, size: impl Into<KeyOrValue<f64>>) -> Self {
        self.set_text_size(size);
        self
    }

    /// Builder-style method for setting the font.
    ///
    /// The argument can be a [`FontDescriptor`] or a [`Key<FontDescriptor>`]
    /// that refers to a font defined in the [`Env`].
    ///
    /// [`Env`]: ../struct.Env.html
    /// [`FontDescriptor`]: ../struct.FontDescriptor.html
    /// [`Key<FontDescriptor>`]: ../struct.Key.html
    pub fn with_font(mut self, font: impl Into<KeyOrValue<FontDescriptor>>) -> Self {
        self.set_font(font);
        self
    }

    /// Builder-style method for setting the text color.
    ///
    /// The argument can be either a `Color` or a [`Key<Color>`].
    ///
    /// [`Key<Color>`]: ../struct.Key.html
    pub fn with_text_color(mut self, color: impl Into<KeyOrValue<Color>>) -> Self {
        self.set_text_color(color);
        self
    }

    /// Set the text size.
    ///
    /// The argument can be either an `f64` or a [`Key<f64>`].
    ///
    /// [`Key<f64>`]: ../struct.Key.html
    pub fn set_text_size(&mut self, size: impl Into<KeyOrValue<f64>>) {
        let size = size.into();
        self.editor.layout_mut().set_text_size(size.clone());
        self.placeholder.set_text_size(size);
    }

    /// Set the font.
    ///
    /// The argument can be a [`FontDescriptor`] or a [`Key<FontDescriptor>`]
    /// that refers to a font defined in the [`Env`].
    ///
    /// [`Env`]: ../struct.Env.html
    /// [`FontDescriptor`]: ../struct.FontDescriptor.html
    /// [`Key<FontDescriptor>`]: ../struct.Key.html
    pub fn set_font(&mut self, font: impl Into<KeyOrValue<FontDescriptor>>) {
        let font = font.into();
        self.editor.layout_mut().set_font(font.clone());
        self.placeholder.set_font(font);
    }

    /// Set the text color.
    ///
    /// The argument can be either a `Color` or a [`Key<Color>`].
    ///
    /// If you change this property, you are responsible for calling
    /// [`request_layout`] to ensure the label is updated.
    ///
    /// [`request_layout`]: ../struct.EventCtx.html#method.request_layout
    /// [`Key<Color>`]: ../struct.Key.html
    pub fn set_text_color(&mut self, color: impl Into<KeyOrValue<Color>>) {
        self.editor.layout_mut().set_text_color(color);
    }

    /// Return the [`Editor`] used by this `TextBox`.
    ///
    /// This is only needed in advanced cases, such as if you want to customize
    /// the drawing of the text.
    pub fn editor(&self) -> &Editor<T> {
        &self.editor
    }

    /// Return a mutable reference to the [`Editor`] used by this `TextBox`.
    ///
    /// [`Editor`]: crate::text::Editor
    //TODO: document the ways you should and shouldn't use this
    pub fn editor_mut(&mut self) -> &mut Editor<T> {
        &mut self.editor
    }
}

impl TextBox<String> {
    /// Turn this `TextBox` into a [`ValueTextBox`], using the [`Formatter`] to
    /// manage the value.
    ///
    /// [`ValueTextBox`]: ValueTextBox
    /// [`Formatter`]: crate::text::format::Formatter
    pub fn with_formatter<T>(self, formatter: impl Formatter<T> + 'static) -> ValueTextBox<T> {
        ValueTextBox::new(self, formatter)
    }
}

impl<T: TextStorage + EditableText> TextBox<T> {
    /// Set the text and force the editor to update.
    //FIXME: do we need this? can we not just rely on `update`?
    pub fn force_rebuild(&mut self, text: T, factory: &mut PietText, env: &Env) {
        self.editor.set_text(text);
        self.editor.rebuild_if_needed(factory, env);
    }

    /// Calculate a stateful scroll offset
    fn update_hscroll(&mut self, self_width: f64) {
        let cursor_x = self.editor.cursor_line().p0.x;
        let overall_text_width = self.editor.layout().size().width;

        //// when advancing the cursor, we want some additional padding
        let padding = TEXT_INSETS.x0 * 2.;
        if overall_text_width < self_width - padding {
            // There's no offset if text is smaller than text box
            //
            // [***I*  ]
            // ^
            self.hscroll_offset = 0.;
        } else if cursor_x > self_width + self.hscroll_offset - padding {
            // If cursor goes past right side, bump the offset
            //       ->
            // **[****I]****
            //   ^
            self.hscroll_offset = cursor_x - self_width + padding;
        } else if cursor_x < self.hscroll_offset {
            // If cursor goes past left side, match the offset
            //    <-
            // **[I****]****
            //   ^
            self.hscroll_offset = cursor_x
        }
    }

    fn reset_cursor_blink(&mut self, token: TimerToken) {
        self.cursor_on = true;
        self.cursor_timer = token;
    }

    // on macos we only draw the cursor if the selection is non-caret
    #[cfg(target_os = "macos")]
    fn should_draw_cursor(&self) -> bool {
        self.cursor_on && self.editor.selection().is_caret()
    }

    #[cfg(not(target_os = "macos"))]
    fn should_draw_cursor(&self) -> bool {
        self.cursor_on
    }
}

impl<T: TextStorage + EditableText> Widget<T> for TextBox<T> {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut T, _env: &Env) {
        self.suppress_adjust_hscroll = false;
        match event {
            Event::MouseDown(mouse) => {
                ctx.request_focus();
                ctx.set_active(true);
                let mut mouse = mouse.clone();
                mouse.pos += Vec2::new(self.hscroll_offset, 0.0);

                if !mouse.focus {
                    self.was_focused_from_click = true;
                    self.reset_cursor_blink(ctx.request_timer(CURSOR_BLINK_DURATION));
                    self.editor.click(&mouse, data);
                }

                ctx.request_paint();
            }
            Event::MouseMove(mouse) => {
                let mut mouse = mouse.clone();
                mouse.pos += Vec2::new(self.hscroll_offset, 0.0);
                ctx.set_cursor(&Cursor::IBeam);
                if ctx.is_active() {
                    self.editor.drag(&mouse, data);
                    ctx.request_paint();
                }
            }
            Event::MouseUp(_) => {
                if ctx.is_active() {
                    ctx.set_active(false);
                    ctx.request_paint();
                }
            }
            Event::Timer(id) => {
                if *id == self.cursor_timer {
                    self.cursor_on = !self.cursor_on;
                    ctx.request_paint();
                    self.cursor_timer = ctx.request_timer(CURSOR_BLINK_DURATION);
                }
            }
            Event::Command(ref cmd) if ctx.is_focused() && cmd.is(crate::commands::COPY) => {
                self.editor.copy(data);
                ctx.set_handled();
            }
            Event::Command(ref cmd) if ctx.is_focused() && cmd.is(crate::commands::CUT) => {
                self.editor.cut(data);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(TextBox::PERFORM_EDIT) => {
                let edit = cmd.get_unchecked(TextBox::PERFORM_EDIT);
                self.editor.do_edit(edit.to_owned(), data);
            }
            Event::Paste(ref item) => {
                if let Some(string) = item.get_string() {
                    self.editor.paste(string, data);
                }
            }
            Event::KeyDown(key_event) => {
                match key_event {
                    // Tab and shift+tab
                    k_e if HotKey::new(None, KbKey::Tab).matches(k_e) => ctx.focus_next(),
                    k_e if HotKey::new(SysMods::Shift, KbKey::Tab).matches(k_e) => ctx.focus_prev(),
                    k_e => {
                        if let Some(edit) = self.input_handler.handle_event(k_e) {
                            self.suppress_adjust_hscroll = matches!(edit, EditAction::SelectAll);
                            self.editor.do_edit(edit, data);
                            // an explicit request update in case the selection
                            // state has changed, but the data hasn't.
                            ctx.request_update();
                            ctx.request_paint();
                        }
                    }
                };
                self.reset_cursor_blink(ctx.request_timer(CURSOR_BLINK_DURATION));
                ctx.request_paint();
            }
            _ => (),
        }
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &T, env: &Env) {
        match event {
            LifeCycle::WidgetAdded => {
                ctx.register_for_focus();
                self.editor.set_text(data.to_owned());
                self.editor.rebuild_if_needed(ctx.text(), env);
            }
            LifeCycle::FocusChanged(is_focused) => {
                if MAC_OR_LINUX && *is_focused && !self.was_focused_from_click {
                    self.editor.select_all(data);
                }
                self.was_focused_from_click = false;
                self.reset_cursor_blink(ctx.request_timer(CURSOR_BLINK_DURATION));
                ctx.request_paint();
            }
            _ => (),
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx, _: &T, data: &T, env: &Env) {
        self.editor.update(ctx, data, env);
        if !self.suppress_adjust_hscroll && !self.multiline {
            self.update_hscroll(ctx.size().width);
        }
        if ctx.env_changed() && self.placeholder.needs_rebuild_after_update(ctx) {
            ctx.request_layout();
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, _data: &T, env: &Env) -> Size {
        let width = env.get(theme::WIDE_WIDGET_WIDTH);

        self.placeholder.rebuild_if_needed(ctx.text(), env);
        if self.multiline {
            self.editor
                .set_wrap_width(bc.max().width - TEXT_INSETS.x_value());
        }
        self.editor.rebuild_if_needed(ctx.text(), env);

        let text_metrics = self.editor.layout().layout_metrics();
        let height = text_metrics.size.height + TEXT_INSETS.y_value();

        let size = bc.constrain((width, height));
        let bottom_padding = (size.height - text_metrics.size.height) / 2.0;
        let baseline_off =
            bottom_padding + (text_metrics.size.height - text_metrics.first_baseline);
        ctx.set_baseline_offset(baseline_off);

        size
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &T, env: &Env) {
        let size = ctx.size();
        let background_color = env.get(theme::BACKGROUND_LIGHT);
        let selection_color = env.get(theme::SELECTION_COLOR);
        let cursor_color = env.get(theme::CURSOR_COLOR);

        let is_focused = ctx.is_focused();

        let border_color = if is_focused {
            env.get(theme::PRIMARY_LIGHT)
        } else {
            env.get(theme::BORDER_DARK)
        };

        // Paint the background
        let clip_rect = Size::new(size.width - BORDER_WIDTH, size.height)
            .to_rect()
            .inset(-BORDER_WIDTH / 2.0)
            .to_rounded_rect(env.get(theme::TEXTBOX_BORDER_RADIUS));

        ctx.fill(clip_rect, &background_color);

        // Render text, selection, and cursor inside a clip
        ctx.with_save(|rc| {
            rc.clip(clip_rect);

            // Shift everything inside the clip by the hscroll_offset
            rc.transform(Affine::translate((-self.hscroll_offset, 0.)));

            let text_pos = Point::new(TEXT_INSETS.x0, TEXT_INSETS.y0);

            // Draw selection rect
            if !data.is_empty() {
                if is_focused {
                    for sel in self.editor.selection_rects() {
                        let sel = sel + text_pos.to_vec2();
                        let rounded = sel.to_rounded_rect(1.0);
                        rc.fill(rounded, &selection_color);
                    }
                }
                self.editor.draw(rc, text_pos);
            } else {
                self.placeholder.draw(rc, text_pos);
            }

            // Paint the cursor if focused and there's no selection
            if is_focused && self.should_draw_cursor() {
                // the cursor position can extend past the edge of the layout
                // (commonly when there is trailing whitespace) so we clamp it
                // to the right edge.
                let mut cursor = self.editor.cursor_line() + text_pos.to_vec2();
                let dx = size.width + self.hscroll_offset - TEXT_INSETS.x_value() - cursor.p0.x;
                if dx < 0.0 {
                    cursor = cursor + Vec2::new(dx, 0.);
                }
                rc.stroke(cursor, &cursor_color, 1.);
            }
        });

        // Paint the border
        ctx.stroke(clip_rect, &border_color, BORDER_WIDTH);
    }
}

impl<T> ValueTextBox<T> {
    /// Create a new `ValueTextBox` from a normal [`TextBox`] and a [`Formatter`].
    ///
    /// [`TextBox`]: crate::widget::TextBox
    /// [`Formatter`]: crate::text::Formatter
    pub fn new(inner: TextBox<String>, formatter: impl Formatter<T> + 'static) -> Self {
        ValueTextBox {
            inner,
            formatter: Box::new(formatter),
            is_editing: false,
            old_buffer: String::new(),
            buffer: String::new(),
            force_selection: None,
        }
    }

    fn complete(&mut self, ctx: &mut EventCtx, data: &mut T, env: &Env) {
        if let Ok(new) = self.formatter.value(&self.buffer) {
            *data = new;
            self.inner
                .force_rebuild(self.formatter.format(data), ctx.text(), env);
            self.is_editing = false;
            ctx.request_layout();
            if ctx.has_focus() {
                ctx.resign_focus();
            }
        } else {
            // don't tab away from here if we're editing
            if !ctx.has_focus() {
                ctx.request_focus();
            }
            ctx.submit_command(
                TextBox::PERFORM_EDIT
                    .with(EditAction::SelectAll)
                    .to(ctx.widget_id()),
            );
            // our content isn't valid
            // ideally we would flash the background or something
        }
    }

    fn cancel(&mut self, ctx: &mut EventCtx, data: &T, env: &Env) {
        self.is_editing = false;
        self.buffer = self.formatter.format(data);
        ctx.request_layout();
        ctx.resign_focus();
        self.inner
            .force_rebuild(self.buffer.clone(), ctx.text(), env);
    }

    fn begin(&mut self, ctx: &mut PietText, data: &T, env: &Env) {
        self.is_editing = true;
        self.buffer = self.formatter.format_for_editing(data);
        self.inner.force_rebuild(self.buffer.clone(), ctx, env);
        self.old_buffer = self.buffer.clone();
    }
}

impl<T: Data> Widget<T> for ValueTextBox<T> {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut T, env: &Env) {
        if self.is_editing {
            // if we reject an edit we want to reset the selection
            let pre_sel = *self.inner.editor().selection();
            match event {
                Event::Command(cmd) if cmd.is(COMPLETE_EDITING) => {
                    return self.complete(ctx, data, env)
                }
                Event::Command(cmd) if cmd.is(CANCEL_EDITING) => {
                    return self.cancel(ctx, data, env)
                }
                Event::KeyDown(k_e) if HotKey::new(None, KbKey::Enter).matches(k_e) => {
                    ctx.set_handled();
                    self.complete(ctx, data, env);
                    return;
                }
                Event::KeyDown(k_e) if HotKey::new(None, KbKey::Escape).matches(k_e) => {
                    ctx.set_handled();
                    self.cancel(ctx, data, env);
                    return;
                }
                event => {
                    self.inner.event(ctx, event, &mut self.buffer, env);
                    ctx.request_paint();
                }
            }
            // if an edit occured, validate it with the formatter
            if self.buffer != self.old_buffer {
                let mut validation = self
                    .formatter
                    .validate_partial_input(&self.buffer, &self.inner.editor().selection());

                let new_buf = match (validation.text_change.take(), validation.is_err()) {
                    (Some(new_text), _) => {
                        // be helpful: if the formatter is misbehaved, log it.
                        if self
                            .formatter
                            .validate_partial_input(&new_text, &Selection::caret(0))
                            .is_err()
                        {
                            log::warn!(
                                "formatter replacement text does not validate: '{}'",
                                &new_text
                            );
                            None
                        } else {
                            Some(new_text)
                        }
                    }
                    (None, true) => Some(self.old_buffer.clone()),
                    _ => None,
                };

                let new_sel = match (validation.selection_change.take(), validation.is_err()) {
                    (Some(new_sel), _) => Some(new_sel),
                    (None, true) => Some(pre_sel),
                    _ => None,
                };

                if let Some(new_buf) = new_buf {
                    self.buffer = new_buf.clone();
                    self.inner.editor_mut().set_text(new_buf);
                }

                //FIXME we stash this and set it in update; can we do the same with `new_buf`?
                self.force_selection = new_sel;
            }
            //TODO: what do we do with result?
            //sure wish we could somehow send a notification up to a parent that
            //wanted to display it, somehow... :thinking-face-emoji:
            ctx.request_update();
        } else if let Event::MouseDown(_) = event {
            self.begin(ctx.text(), data, env);
            self.inner.event(ctx, event, &mut self.buffer, env);
        }
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &T, env: &Env) {
        if let LifeCycle::WidgetAdded = event {
            self.buffer = self.formatter.format(data);
            self.old_buffer = self.buffer.clone();
        }
        self.inner.lifecycle(ctx, event, &self.buffer, env);

        if let LifeCycle::FocusChanged(focus) = event {
            // if the user focuses elsewhere, we need to reset ourselves
            if !focus {
                ctx.submit_command(COMPLETE_EDITING.to(ctx.widget_id()));
            } else if !self.is_editing {
                self.begin(ctx.text(), data, env);
                ctx.request_layout();
            }
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx, old_data: &T, data: &T, env: &Env) {
        let in_edit_mode = self.is_editing && data.same(old_data);
        if in_edit_mode {
            self.inner.update(ctx, &self.old_buffer, &self.buffer, env);
            self.old_buffer = self.buffer.clone();
        } else if !data.same(old_data) {
            let new_text = self.formatter.format(data);
            self.old_buffer = std::mem::replace(&mut self.buffer, new_text);
            self.inner.update(ctx, &self.old_buffer, &self.buffer, env);
            if self.is_editing {
                //TODO data changed externally, cancel editing
            }
        } else if ctx.env_changed() {
            self.inner.update(ctx, &self.buffer, &self.buffer, env);
            ctx.request_layout();
        }
        if let Some(sel) = self.force_selection.take() {
            self.inner.editor_mut().set_selection(sel);
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, _data: &T, env: &Env) -> Size {
        self.inner.layout(ctx, bc, &self.buffer, env)
    }

    fn paint(&mut self, ctx: &mut PaintCtx, _data: &T, env: &Env) {
        self.inner.paint(ctx, &self.buffer, env);
    }
}

impl<T> Default for TextBox<T> {
    fn default() -> Self {
        TextBox::new()
    }
}

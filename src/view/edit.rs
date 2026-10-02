//! 아래쪽 패널의 파일 편집기와, 편집을 떠날 때 묻는 확인 창
//! 편집기는 줄 번호 + 고정폭 입력칸이다. 줄을 접지 않고 가로로 스크롤한다. ⌘S로 저장, ⌘Z로 되돌리기.

use std::sync::Arc;

use eframe::egui::{
    self, Align, FontId, Galley, Id, Key, Layout, Margin, Modifiers, RichText, ScrollArea, TextBuffer, TextEdit, vec2,
};
use eframe::epaint::text::LayoutJob;

use crate::app::{App, Leave};
use crate::edit::Editor;
use crate::style::{self, LABEL, Palette, TITLE};

/// `locked`면 확인 창이 떠 있는 동안이라 입력을 받지 않는다 (Enter가 편집기로 새지 않게).
pub fn body(ui: &mut egui::Ui, ed: &mut Editor, pal: &Palette, locked: bool) {
    let mono = FontId::monospace(LABEL);
    let rows = ed.text.matches('\n').count() + 1;
    let digits = rows.to_string().len().max(3);
    let numbers: String = (1..=rows).map(|n| format!("{n:>digits$}\n")).collect();
    let color = pal.text;
    let height = ui.available_height();

    ScrollArea::both().id_salt(("edit", &ed.path)).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(12.0);
            let gutter = RichText::new(numbers.trim_end()).font(mono.clone()).color(pal.weak);
            ui.add(egui::Label::new(gutter).selectable(false).extend());
            ui.add_space(4.0);
            // 줄을 접지 않는다 (줄 번호와 행이 어긋나지 않게).
            let mut layouter = |ui: &egui::Ui, text: &dyn TextBuffer, _wrap: f32| -> Arc<Galley> {
                let job = LayoutJob::simple(text.as_str().to_owned(), mono.clone(), color, f32::INFINITY);
                ui.ctx().fonts_mut(|f| f.layout_job(job))
            };
            let edit = TextEdit::multiline(&mut ed.text)
                .id(Id::new(("editor", &ed.path)))
                .code_editor()
                .lock_focus(true)
                .interactive(!locked)
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(f32::INFINITY)
                .min_size(vec2(0.0, height))
                .layouter(&mut layouter);
            ui.add(edit);
        });
    });
}

/// 저장 안 한 편집을 두고 떠나려 할 때, 그리고 다른 곳에서 바뀐 파일을 덮어쓰려 할 때 묻는다.
pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let name = app.editor.as_ref().map(|e| e.path.rsplit('/').next().unwrap_or(&e.path).to_string());
    let Some(name) = name else {
        app.leaving = None;
        return;
    };

    if app.leaving.is_some() {
        let quit = matches!(app.leaving, Some(Leave::Quit));
        let (title, body) = ("저장하지 않은 변경이 있어요", format!("{name}에 고친 내용을 저장할까요?"));
        let (choice, close) = ask(ctx, "leave", title, &body, &["저장", if quit { "저장 안 하고 끄기" } else { "저장 안 함" }, "취소"]);
        match choice.or(close.then_some(2)) {
            Some(0) => {
                // 저장에 실패하면(다른 곳에서 바뀌었으면) 머무르고 그 확인 창을 띄운다.
                let to = app.leaving.take();
                if let (true, Some(to)) = (app.save_edit(false), to) {
                    app.go(to);
                }
            }
            Some(1) => {
                if let Some(to) = app.leaving.take() {
                    app.go(to);
                }
            }
            Some(_) => app.leaving = None,
            None => {}
        }
        return;
    }

    if app.editor.as_ref().is_some_and(|e| e.conflict) {
        let body = format!("{name}을(를) 편집하는 동안 다른 곳에서 파일이 바뀌었어요.\n저장하면 그 변경을 덮어써요.");
        let (choice, close) = ask(ctx, "overwrite", "파일이 바뀌었어요", &body, &["덮어쓰기", "취소"]);
        match choice.or(close.then_some(1)) {
            Some(0) => {
                app.save_edit(true);
            }
            Some(_) => {
                if let Some(e) = app.editor.as_mut() {
                    e.conflict = false;
                }
            }
            None => {}
        }
    }
}

/// 버튼을 오른쪽부터 늘어놓은 확인 창. 누른 버튼 번호와, Esc·바깥 클릭으로 닫혔는지를 돌려준다.
/// Enter는 첫 번째 버튼이다.
fn ask(ctx: &egui::Context, id: &str, title: &str, body: &str, buttons: &[&str]) -> (Option<usize>, bool) {
    let modal = egui::Modal::new(Id::new(("ask", id))).show(ctx, |ui| {
        let pal = Palette::of(ui);
        ui.set_width(380.0);
        ui.label(RichText::new(title).font(style::bold(TITLE)).color(pal.text));
        ui.add_space(4.0);
        ui.label(RichText::new(body).color(pal.text));
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            buttons.iter().position(|b| ui.button(*b).clicked())
        })
        .inner
    });
    let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let choice = modal.inner.or(enter.then_some(0));
    (choice, choice.is_none() && modal.should_close())
}

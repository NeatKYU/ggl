//! 오른쪽 diff 패널: 상세의 파일 목록에서 누른 파일의 변경 내용
//! 줄 번호 칸은 가로로 스크롤해도 왼쪽에 고정된다.

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, ScrollArea, Sense, pos2, vec2};

use crate::app::App;
use crate::git::{Diff, LineKind};
use crate::style::{self, LABEL, Palette, SMALL, TEXT};
use crate::view::details::status_style;

const LINE_H: f32 = 18.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    // 패널은 그려진 내용 크기를 기억하므로, 불러오는 중에도 항상 꽉 채워서 크기가 줄지 않게 한다.
    ui.set_min_size(ui.available_size());
    let pal = Palette::of(ui);
    let Some(view) = &app.diff else { return };
    let file = view.file.clone();
    let index = view.index;
    let count = app.file_count();

    // 머리: 상태 · 파일 이름 · 폴더 · +/− · 이전/다음/닫기
    let mut go = None;
    let mut close = false;
    egui::Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        // 버튼을 오른쪽부터 먼저 놓고, 남은 폭에 파일 이름을 말줄임으로 넣는다.
        // (한 줄 높이로 감싸지 않으면 오른쪽 정렬 레이아웃이 패널 높이를 다 차지한다)
        ui.horizontal(|ui| ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("닫기").on_hover_text("Esc").clicked() {
                close = true;
            }
            if ui.add_enabled(index + 1 < count, egui::Button::new("다음 ▸")).clicked() {
                go = Some(index + 1);
            }
            if ui.add_enabled(index > 0, egui::Button::new("◂ 이전")).clicked() {
                go = index.checked_sub(1);
            }
            ui.label(RichText::new(format!("{}/{}", index + 1, count)).small().color(pal.weak));
            if let (Some(a), Some(d)) = (file.added, file.deleted) {
                ui.label(RichText::new(format!("−{d}")).small().color(pal.deleted));
                ui.label(RichText::new(format!("+{a}")).small().color(pal.added));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let (letter, color, hint) = status_style(file.status, &pal);
                ui.label(RichText::new(letter).monospace().color(color)).on_hover_text(hint);
                let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
                let name = egui::Label::new(RichText::new(name).font(style::bold(TEXT)).color(pal.text)).truncate();
                ui.add(name).on_hover_text(&file.path);
                let mut sub = dir.to_string();
                if let Some(old) = &file.old_path {
                    sub = format!("{sub}  ← {old}");
                }
                if !sub.trim().is_empty() {
                    ui.add(egui::Label::new(RichText::new(sub.trim()).small().color(pal.weak)).truncate());
                }
            });
        }));
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().hline(sep.x_range(), sep.top(), egui::Stroke::new(1.0, pal.border));

    if close {
        app.diff = None;
        return;
    }
    if let Some(i) = go {
        app.open_diff(i);
        return;
    }

    let Some(view) = &app.diff else { return };
    let message = |ui: &mut egui::Ui, text: &str, color: Color32| {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(text).color(color)));
    };
    match &view.result {
        None => message(ui, "불러오는 중…", pal.weak),
        Some(Err(e)) => message(ui, &format!("변경 내용을 읽지 못했어요: {e}"), pal.deleted),
        Some(Ok(d)) if d.binary => message(ui, "바이너리 파일이라 내용을 보여줄 수 없어요", pal.weak),
        Some(Ok(d)) if d.lines.is_empty() && d.truncated => {
            message(ui, "파일이 너무 커서(2MB 초과) 보여주지 않아요", pal.weak)
        }
        Some(Ok(d)) if d.lines.is_empty() => message(ui, "내용 변경은 없어요 (이름이나 권한만 바뀜)", pal.weak),
        Some(Ok(d)) => {
            let id = egui::Id::new(("diff", &view.hash, &view.file.path));
            lines(ui, d, id, &pal);
        }
    }
}

/// 줄 목록. 파일 내용 그대로(`plain`)면 줄 번호 칸이 하나다.
pub fn lines(ui: &mut egui::Ui, d: &Diff, id: egui::Id, pal: &Palette) {
    let mono = FontId::monospace(LABEL);
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&mono, '0'));
    let max_no = d.lines.iter().map(|l| l.old.max(l.new)).max().unwrap_or(0);
    let digits = max_no.to_string().len().max(3) as f32;
    let num_w = digits * char_w + 14.0;
    let gutter_w = if d.plain { num_w + 10.0 } else { num_w * 2.0 + 18.0 };
    let content_w = (gutter_w + d.max_cols as f32 * char_w + 40.0).max(ui.available_width());
    let gutter_bg = if pal.bg.r() < 128 { Color32::from_gray(0x23) } else { Color32::from_gray(0xf6) };

    ui.spacing_mut().item_spacing.y = 0.0;
    let n = d.lines.len() + usize::from(d.truncated);
    ScrollArea::both().id_salt(id).auto_shrink([false, false]).show_rows(ui, LINE_H, n, |ui, range| {
        let clip = ui.clip_rect();
        let p = ui.painter().clone();
        for i in range {
            let (rect, _) = ui.allocate_exact_size(vec2(content_w, LINE_H), Sense::hover());
            let cy = rect.center().y;
            let Some(l) = d.lines.get(i) else {
                let text = format!("… 너무 길어서 {}줄까지만 보여줘요", d.lines.len());
                p.text(pos2(rect.left() + gutter_w, cy), Align2::LEFT_CENTER, text, FontId::proportional(SMALL), pal.weak);
                continue;
            };
            let (bg, marker, color) = match l.kind {
                LineKind::Added => (Some(pal.diff_add_bg), "+", pal.text),
                LineKind::Deleted => (Some(pal.diff_del_bg), "−", pal.text),
                LineKind::Hunk => (Some(pal.diff_hunk_bg), "", pal.diff_hunk),
                LineKind::Meta => (None, "", pal.weak),
                LineKind::Context => (None, "", pal.text),
            };
            if let Some(bg) = bg {
                p.rect_filled(rect, 0.0, bg);
            }
            p.text(pos2(rect.left() + gutter_w, cy), Align2::LEFT_CENTER, &l.text, mono.clone(), color);

            // 고정된 줄 번호 칸
            let g = Rect::from_min_size(pos2(clip.left(), rect.top()), vec2(gutter_w, LINE_H));
            p.rect_filled(g, 0.0, gutter_bg);
            if let Some(bg) = bg {
                p.rect_filled(g, 0.0, bg);
            }
            let num = |n: u32, x: f32| {
                if n > 0 {
                    p.text(pos2(x, cy), Align2::RIGHT_CENTER, n.to_string(), mono.clone(), pal.weak);
                }
            };
            if d.plain {
                num(l.new, g.left() + num_w - 6.0);
                continue;
            }
            num(l.old, g.left() + num_w - 6.0);
            num(l.new, g.left() + num_w * 2.0 - 6.0);
            p.text(pos2(g.left() + num_w * 2.0 + 4.0, cy), Align2::LEFT_CENTER, marker, mono.clone(), pal.weak);
        }
    });
}

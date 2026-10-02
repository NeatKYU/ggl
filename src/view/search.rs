//! 왼쪽 패널의 검색 탭 (⌘⇧F): 저장소 파일 내용에서 글자를 찾는다.
//! 위에는 이름이 맞는 파일, 아래에는 내용이 맞는 줄을 파일별로 보여준다. 줄을 누르면 그 줄로 파일을 연다.
//! 입력을 잠깐 멈추면 찾고, 파일이 바뀌면 다시 찾는다.

use std::borrow::Cow;
use std::ops::Range;

use eframe::egui::{
    self, Align, Align2, CornerRadius, CursorIcon, FontId, Key, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    TextEdit, pos2, vec2,
};
use eframe::epaint::text::{LayoutJob, TextFormat, TextWrapping};

use crate::app::App;
use crate::git::{Grep, MAX_GREP_LINES};
use crate::style::{self, LABEL, Palette, SMALL, TEXT, icon, icon_toggle};
use crate::view::table::truncated;

const ROW_H: f32 = 22.0;
const NUM_W: f32 = 44.0;

enum Item {
    Title(String),
    /// 이름이 맞는 파일 (트리 노드 번호)
    Name(usize),
    /// 내용이 맞는 파일 (검색 결과의 파일 번호)
    File(usize),
    Line(usize, usize),
    More,
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let pal = Palette::of(ui);
    egui::Frame::new().inner_margin(Margin { left: 10, right: 8, top: 4, bottom: 6 }).show(ui, |ui| {
        // 한 줄 높이로 감싸지 않으면 오른쪽 정렬 레이아웃이 패널 높이를 다 차지한다.
        ui.horizontal(|ui| ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let case = ui.add(icon_toggle(app.find.case, icon::CASE));
            if case.on_hover_text("대소문자 구분").clicked() {
                app.find.case = !app.find.case;
                app.run_find();
            }
            let edit = TextEdit::singleline(&mut app.find.query)
                .hint_text("내용 검색  ⌘⇧F")
                .desired_width(ui.available_width());
            let r = ui.add(edit);
            if std::mem::take(&mut app.find.focus) {
                r.request_focus();
            }
            if r.changed() {
                app.find_changed();
            }
            if r.lost_focus() {
                let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if esc {
                    app.find.query.clear();
                    app.find_changed();
                    app.run_find();
                } else if enter {
                    app.run_find();
                    r.request_focus();
                }
            }
        }));
        ui.add_space(2.0);
        ui.horizontal(|ui| status(app, ui, &pal));
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().hline(sep.x_range(), sep.top(), Stroke::new(1.0, pal.border));

    let grep = match &app.find.result {
        Some(Ok(g)) => Some(g),
        _ => None,
    };
    let items = items(app, grep);
    if items.is_empty() {
        return;
    }
    let query = app.find.ran.clone().unwrap_or_default();
    let tree = match &app.tree {
        Some(Ok(t)) => Some(t),
        _ => None,
    };
    let open = app.file_view.as_ref().map(|v| (v.path.as_str(), v.mark));
    let mut clicked: Option<(String, Option<u32>)> = None;

    ui.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical().id_salt("find-results").auto_shrink([false, false]).show_rows(ui, ROW_H, items.len(), |ui, range| {
        let width = ui.available_width();
        for item in &items[range] {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let p = ui.painter();
            let cy = rect.center().y;
            let (path, line) = match item {
                Item::Title(text) => {
                    p.text(pos2(rect.left() + 10.0, cy + 2.0), Align2::LEFT_CENTER, text, style::bold(SMALL), pal.weak);
                    continue;
                }
                Item::More => {
                    let text = format!("… {MAX_GREP_LINES}줄까지만 보여줘요. 더 자세히 입력해 보세요");
                    p.text(pos2(rect.left() + 10.0, cy), Align2::LEFT_CENTER, text, FontId::proportional(SMALL), pal.weak);
                    continue;
                }
                Item::Name(i) => match tree {
                    Some(t) => (t.nodes[*i].path.as_str(), None),
                    None => continue,
                },
                Item::File(f) => {
                    let file = &grep.unwrap().files[*f];
                    (file.path.as_str(), file.lines.first().map(|l| l.line))
                }
                Item::Line(f, l) => {
                    let file = &grep.unwrap().files[*f];
                    (file.path.as_str(), Some(file.lines[*l].line))
                }
            };
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            let selected = matches!(item, Item::Line(..)) && open == Some((path, line));
            if selected {
                p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), CornerRadius::same(3), pal.selected);
            } else if resp.hovered() {
                p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), CornerRadius::same(3), pal.hover);
            }
            match item {
                Item::Line(f, l) => {
                    let hit = &grep.unwrap().files[*f].lines[*l];
                    let num_r = rect.left() + 10.0 + NUM_W;
                    let mono = FontId::monospace(SMALL);
                    p.text(pos2(num_r, cy), Align2::RIGHT_CENTER, hit.line.to_string(), mono, pal.weak);
                    let x = num_r + 10.0;
                    let job = highlighted(&hit.text, &query, rect.right() - 10.0 - x, &pal);
                    let g = p.layout_job(job);
                    p.galley(pos2(x, cy - g.size().y / 2.0), g, pal.text);
                }
                Item::File(f) => {
                    let count = grep.unwrap().files[*f].lines.len().to_string();
                    let r = p.text(pos2(rect.right() - 10.0, cy), Align2::RIGHT_CENTER, count, FontId::proportional(SMALL), pal.weak);
                    name_and_folder(p, rect.left() + 10.0, r.left() - 8.0, cy, path, true, &pal);
                }
                _ => name_and_folder(p, rect.left() + 10.0, rect.right() - 10.0, cy, path, false, &pal),
            }
            let hint = match line {
                Some(n) if matches!(item, Item::Line(..)) => format!("{path}:{n}"),
                _ => path.to_string(),
            };
            if resp.on_hover_text(hint).clicked() {
                clicked = Some((path.to_string(), line));
            }
        }
    });

    if let Some((path, line)) = clicked {
        let status = app.file_status(&path);
        app.open_file(&path, status, line);
    }
}

/// 입력칸 아래 한 줄: 찾는 중, 결과 수, 오류
fn status(app: &App, ui: &mut egui::Ui, pal: &Palette) {
    let small = |text: String, color| RichText::new(text).small().color(color);
    if app.find.running {
        ui.add(egui::Spinner::new().size(11.0));
        ui.label(small("찾는 중…".into(), pal.weak));
        return;
    }
    match &app.find.result {
        Some(Err(e)) => {
            ui.label(small(format!("⚠ {e}"), pal.deleted));
        }
        Some(Ok(g)) if g.lines == 0 => {
            ui.label(small("내용이 맞는 줄이 없어요".into(), pal.weak));
        }
        Some(Ok(g)) => {
            let more = if g.truncated { "+" } else { "" };
            ui.label(small(format!("{}개 파일 · {}줄{more}", g.files.len(), g.lines), pal.weak));
        }
        None => {
            ui.label(small("저장소 파일의 내용에서 찾아요".into(), pal.weak));
        }
    }
}

fn items(app: &App, grep: Option<&Grep>) -> Vec<Item> {
    let mut items = Vec::new();
    if !app.find.names.is_empty() {
        items.push(Item::Title("이름이 맞는 파일".into()));
        items.extend(app.find.names.iter().map(|&i| Item::Name(i)));
    }
    if let Some(g) = grep.filter(|g| !g.files.is_empty()) {
        items.push(Item::Title("내용".into()));
        for (f, file) in g.files.iter().enumerate() {
            items.push(Item::File(f));
            items.extend((0..file.lines.len()).map(|l| Item::Line(f, l)));
        }
        if g.truncated {
            items.push(Item::More);
        }
    }
    items
}

fn name_and_folder(p: &egui::Painter, x: f32, right: f32, cy: f32, path: &str, bold: bool, pal: &Palette) {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let font = if bold { style::bold(TEXT) } else { FontId::proportional(TEXT) };
    let g = truncated(p, name, font, pal.text, right - x, false);
    let name_w = g.size().x;
    p.galley(pos2(x, cy - g.size().y / 2.0), g, pal.text);
    let dx = x + name_w + 6.0;
    if !dir.is_empty() && right - dx > 24.0 {
        let g = truncated(p, dir, FontId::proportional(SMALL), pal.weak, right - dx, false);
        p.galley(pos2(dx, cy - g.size().y / 2.0), g, pal.weak);
    }
}

/// 맞은 글자에 배경을 칠한 한 줄 (폭을 넘으면 말줄임)
fn highlighted(text: &str, query: &(String, bool), width: f32, pal: &Palette) -> LayoutJob {
    let normal = TextFormat { font_id: FontId::monospace(LABEL), color: pal.text, ..Default::default() };
    let hit = TextFormat { background: pal.found.gamma_multiply(2.5), ..normal.clone() };
    let mut job = LayoutJob::default();
    let mut at = 0;
    for r in matches(text, &query.0, query.1) {
        job.append(&text[at..r.start], 0.0, normal.clone());
        job.append(&text[r.clone()], 0.0, hit.clone());
        at = r.end;
    }
    job.append(&text[at..], 0.0, normal);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    job
}

/// `text`에서 `query`가 나오는 곳 (바이트 범위). 소문자로 바꾸면 길이가 달라지는 글자가 있으면 칠하지 않는다.
fn matches(text: &str, query: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, needle): (Cow<str>, Cow<str>) = if case_sensitive {
        (text.into(), query.into())
    } else {
        (text.to_lowercase().into(), query.to_lowercase().into())
    };
    if hay.len() != text.len() {
        return Vec::new();
    }
    hay.match_indices(needle.as_ref())
        .map(|(i, m)| i..i + m.len())
        .filter(|r| text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ranges_to_highlight() {
        assert_eq!(matches("Find and find", "find", false), [0..4, 9..13]);
        assert_eq!(matches("Find and find", "find", true), [9..13]);
        assert_eq!(matches("하루예약 예약", "예약", false), [6..12, 13..19]);
        assert!(matches("abc", "", false).is_empty());
    }
}

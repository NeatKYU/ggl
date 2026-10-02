//! 커밋 상세 (커밋 행 바로 아래에 펼쳐짐)
//! 왼쪽: 메시지·작성자·부모 / 오른쪽: 변경된 파일. 가운데 선을 끌면 넓이가, 아래 테두리를 끌면 높이가 바뀐다.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Rect, RichText,
    ScrollArea, Sense, Stroke, UiBuilder, pos2, vec2,
};

use crate::app::{App, ScrollTo};
use crate::git::{Details, FileChange, UNCOMMITTED};
use crate::style::{self, LABEL, Palette, SMALL, TEXT, TITLE};
use crate::view::table::truncated;

const CONTROLS_W: f32 = 32.0;
const FILE_ROW_H: f32 = 22.0;
const MIN_HEIGHT: f32 = 120.0;

pub enum Action {
    Close,
    Jump(String),
    OpenDiff(usize),
}

/// 드래그로 바뀌는 크기 (호출한 쪽이 설정에 저장한다)
pub struct Inline {
    /// 왼쪽 칸 비율 (0.2 ~ 0.8)
    pub split: f32,
    pub height: f32,
    pub max_height: f32,
}

type Loaded = Option<(String, Result<Details, String>)>;

pub fn inline(
    ui: &mut egui::Ui,
    rect: Rect,
    hash: &str,
    details: &Loaded,
    open_file: Option<usize>,
    state: &mut Inline,
    pal: &Palette,
) -> Option<Action> {
    let mut action = None;
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, pal.inline_bg);
    p.vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, pal.border));

    // 아래 테두리: 끌어서 높이 조절
    let handle = Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 4.0..=rect.bottom() + 2.0);
    let r = ui.interact(handle, Id::new("cdv-height"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
    if r.dragged() {
        state.height = (state.height + r.drag_delta().y).clamp(MIN_HEIGHT, state.max_height);
    }
    let active = r.hovered() || r.dragged();
    p.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, if active { pal.weak } else { pal.border }));

    let content = Rect::from_min_max(rect.min, pos2(rect.right() - CONTROLS_W, rect.bottom() - 2.0));

    // 오른쪽 위 닫기 버튼
    let close = Rect::from_center_size(pos2(rect.right() - CONTROLS_W / 2.0, rect.top() + 16.0), vec2(22.0, 22.0));
    let r = ui.interact(close, Id::new("cdv-close"), Sense::click()).on_hover_text("닫기 (Esc)");
    if r.hovered() {
        p.rect_filled(close, CornerRadius::same(4), pal.hover);
    }
    let (c, k) = (close.center(), 4.5);
    let x_stroke = Stroke::new(1.5, if r.hovered() { pal.text } else { pal.weak });
    p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], x_stroke);
    p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], x_stroke);
    if r.clicked() {
        action = Some(Action::Close);
    }

    let d = match details {
        Some((h, Ok(d))) if h == hash => d,
        Some((h, Err(e))) if h == hash => {
            let text = format!("상세 정보를 읽지 못했어요: {e}");
            p.text(content.center(), Align2::CENTER_CENTER, text, FontId::proportional(TEXT), pal.deleted);
            return action;
        }
        _ => {
            p.text(content.center(), Align2::CENTER_CENTER, "불러오는 중…", FontId::proportional(TEXT), pal.weak);
            return action;
        }
    };

    // 가운데 구분선: 끌어서 왼쪽/오른쪽 넓이 조절
    let split_x = content.left() + content.width() * state.split;
    let divider = Rect::from_x_y_ranges(split_x - 3.0..=split_x + 3.0, content.y_range());
    let r = ui.interact(divider, Id::new("cdv-divider"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
    if r.dragged() {
        let x = split_x + r.drag_delta().x;
        state.split = ((x - content.left()) / content.width()).clamp(0.2, 0.8);
    }
    let active = r.hovered() || r.dragged();
    p.vline(split_x, content.y_range(), Stroke::new(1.0, if active { pal.weak } else { pal.border }));

    let summary_rect = Rect::from_min_max(content.min, pos2(split_x - 3.0, content.bottom())).shrink2(vec2(12.0, 10.0));
    let files_rect = Rect::from_min_max(pos2(split_x + 3.0, content.top()), content.max).shrink2(vec2(6.0, 6.0));

    // 각 칸은 자기 영역 밖으로 그리지 않는다 (긴 해시·이메일이 옆 칸을 덮지 않게).
    let clip = ui.clip_rect();
    let mut child = ui.new_child(UiBuilder::new().max_rect(summary_rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(summary_rect.expand(4.0).intersect(clip));
    ScrollArea::vertical().id_salt(("cdv-summary", hash)).auto_shrink([false, false]).show(&mut child, |ui| {
        if let Some(a) = summary(ui, d, pal) {
            action = Some(a);
        }
    });

    let mut child = ui.new_child(UiBuilder::new().max_rect(files_rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(files_rect.expand(4.0).intersect(clip));
    ScrollArea::vertical().id_salt(("cdv-files", hash)).auto_shrink([false, false]).show(&mut child, |ui| {
        if let Some(i) = files(ui, d, open_file, files_rect.width() - 8.0, pal) {
            action = Some(Action::OpenDiff(i));
        }
    });
    action
}

pub fn apply(app: &mut App, action: Action) {
    match action {
        Action::Close => app.close_details(),
        Action::Jump(hash) => {
            let row = app.data.as_ref().and_then(|d| d.rows.get(&hash).copied());
            app.select(hash);
            if let Some(row) = row {
                app.scroll_to = Some(ScrollTo::Center(row));
            }
        }
        Action::OpenDiff(i) => app.open_diff(i),
    }
}

fn summary(ui: &mut egui::Ui, d: &Details, pal: &Palette) -> Option<Action> {
    let mut action = None;
    let mut lines = d.body.lines();
    let title = lines.next().unwrap_or_default();
    let rest = lines.collect::<Vec<_>>().join("\n");

    ui.add(egui::Label::new(RichText::new(title).font(style::bold(TITLE)).color(pal.text)).selectable(true));
    ui.add_space(6.0);

    if d.hash != UNCOMMITTED {
        let row = |ui: &mut egui::Ui, key: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(62.0, 18.0), Sense::hover());
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, key, FontId::proportional(SMALL), pal.weak);
                add(ui);
            });
        };
        let value = |text: RichText| egui::Label::new(text).selectable(true).truncate();

        row(ui, "커밋", &mut |ui| {
            ui.add(value(RichText::new(&d.hash).monospace()));
        });
        if !d.parents.is_empty() {
            row(ui, if d.parents.len() > 1 { "부모 (머지)" } else { "부모" }, &mut |ui| {
                for p in &d.parents {
                    let short = &p[..p.len().min(7)];
                    let link = ui.link(RichText::new(short).monospace()).on_hover_text("이 커밋으로 이동");
                    if link.clicked() {
                        action = Some(Action::Jump(p.clone()));
                    }
                }
            });
        }
        row(ui, "작성자", &mut |ui| {
            ui.add(value(RichText::new(format!("{} <{}>", d.author, d.email))));
        });
        row(ui, "날짜", &mut |ui| {
            ui.add(value(RichText::new(&d.date)));
        });
        if d.committer != d.author || d.commit_date != d.date {
            row(ui, "커미터", &mut |ui| {
                ui.add(value(RichText::new(format!("{}  ·  {}", d.committer, d.commit_date))));
            });
        }
    }

    let rest = rest.trim();
    if !rest.is_empty() {
        ui.add_space(10.0);
        ui.add(egui::Label::new(RichText::new(rest).color(pal.text)).selectable(true));
    }
    action
}

/// 변경된 파일 목록. 누른 파일의 번호를 돌려준다.
fn files(ui: &mut egui::Ui, d: &Details, open: Option<usize>, width: f32, pal: &Palette) -> Option<usize> {
    let (added, deleted) = d.files.iter().fold((0, 0), |(a, b), f| (a + f.added.unwrap_or(0), b + f.deleted.unwrap_or(0)));
    // 머리줄은 칸 폭 안에서만 그린다 (좁으면 +/−부터 생략).
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let title = format!("변경된 파일 {}개", d.files.len());
    let r = p.text(pos2(rect.left() + 8.0, cy), Align2::LEFT_CENTER, title, style::bold(LABEL), pal.text);
    let small = FontId::proportional(SMALL);
    let mut x = r.right() + 8.0;
    for (text, color) in [(format!("+{added}"), pal.added), (format!("−{deleted}"), pal.deleted)] {
        let g = p.layout_no_wrap(text, small.clone(), color);
        if x + g.size().x > rect.right() - 28.0 {
            break;
        }
        let w = g.size().x;
        p.galley(pos2(x, cy - g.size().y / 2.0), g, color);
        x += w + 6.0;
    }
    if d.files.is_empty() {
        ui.label(RichText::new("  바뀐 파일이 없어요").small().color(pal.weak));
        return None;
    }

    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (i, f) in d.files.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, FILE_ROW_H), Sense::click());
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if open == Some(i) {
            ui.painter().rect_filled(rect, CornerRadius::same(3), pal.selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), pal.hover);
        }
        file_row(ui.painter(), rect, f, pal);
        if resp.clicked() {
            clicked = Some(i);
        }
        let hint = match &f.old_path {
            Some(old) => format!("{old} → {}\n눌러서 변경 내용 보기", f.path),
            None => format!("{}\n눌러서 변경 내용 보기", f.path),
        };
        resp.on_hover_text(hint);
    }
    clicked
}

fn file_row(p: &egui::Painter, rect: Rect, f: &FileChange, pal: &Palette) {
    let cy = rect.center().y;
    let (letter, color, _) = status_style(f.status, pal);
    p.text(pos2(rect.left() + 8.0, cy), Align2::LEFT_CENTER, letter, FontId::monospace(LABEL), color);

    // 오른쪽 +/− 먼저 그리고, 남은 폭에 파일 이름과 폴더를 넣는다.
    let mut right = rect.right() - 8.0;
    let small = FontId::proportional(SMALL);
    let stats: Vec<(String, Color32)> = match (f.added, f.deleted) {
        (Some(a), Some(d)) => vec![(format!("−{d}"), pal.deleted), (format!("+{a}"), pal.added)],
        _ if f.status == 'U' => vec![("새 파일".into(), pal.weak)],
        _ => vec![("바이너리".into(), pal.weak)],
    };
    for (text, color) in stats {
        let r = p.text(pos2(right, cy), Align2::RIGHT_CENTER, text, small.clone(), color);
        right = r.left() - 6.0;
    }

    let (dir, name) = match f.path.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", f.path.as_str()),
    };
    let x = rect.left() + 26.0;
    let g = truncated(p, name, FontId::proportional(TEXT), pal.text, right - x - 8.0, false);
    let name_w = g.size().x;
    p.galley(pos2(x, cy - g.size().y / 2.0), g, pal.text);
    let mut extra = dir.to_string();
    if let Some(old) = &f.old_path {
        extra = format!("{extra}  ← {old}");
    }
    let dx = x + name_w + 8.0;
    if !extra.trim().is_empty() && right - dx > 30.0 {
        let g = truncated(p, extra.trim(), small, pal.weak, right - dx - 8.0, false);
        p.galley(pos2(dx, cy - g.size().y / 2.0), g, pal.weak);
    }
}

pub fn status_style(status: char, pal: &Palette) -> (&'static str, Color32, &'static str) {
    let dark = pal.bg.r() < 128;
    let amber = if dark { Color32::from_rgb(0xe2, 0xc0, 0x8d) } else { Color32::from_rgb(0x89, 0x55, 0x03) };
    let blue = if dark { Color32::from_rgb(0x4f, 0xc1, 0xff) } else { Color32::from_rgb(0x00, 0x70, 0xc1) };
    match status {
        'A' => ("A", pal.added, "추가됨"),
        'D' => ("D", pal.deleted, "삭제됨"),
        'R' => ("R", blue, "이름 바뀜"),
        'C' => ("C", blue, "복사됨"),
        'U' => ("U", pal.added, "새 파일 (추적 안 됨)"),
        'T' => ("T", amber, "형식 바뀜"),
        '!' => ("!", pal.deleted, "충돌"),
        _ => ("M", amber, "수정됨"),
    }
}

//! 빠른 열기 (⌘P): 화면 위쪽에 입력칸을 띄우고, 파일 이름 일부만 쳐도 저장소 파일을 찾아 연다 (VS Code처럼).
//! ↑↓로 고르고 Enter로 연다. Esc나 바깥을 누르면 닫힌다.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Modifiers, RichText, ScrollArea, Sense, TextEdit,
    pos2, vec2,
};
use eframe::epaint::text::{LayoutJob, TextFormat, TextWrapping};

use crate::app::App;
use crate::style::{LABEL, Palette, SMALL, TEXT, icon};
use crate::tree::Node;

const ROW_H: f32 = 26.0;
/// 창이 작아도 이만큼은 보여준다
const MIN_ROWS: f32 = 8.0;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut q) = app.quick.take() else { return };
    // 입력칸보다 먼저 가져간다 (Enter에 입력칸이 포커스를 잃거나, ↑↓가 커서를 옮기지 않게).
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    let count = q.hits.len();
    if count > 0 && up {
        q.sel = q.sel.checked_sub(1).unwrap_or(count - 1);
    }
    if count > 0 && down {
        q.sel = (q.sel + 1) % count;
    }

    let id = Id::new("quick-open");
    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).clamp(280.0, 620.0);
    // 목록은 창 높이에 맞춰 아래로 길게 (위쪽 52px 아래 남은 높이의 80%에서 입력칸 몫을 뺀 만큼)
    let list_h = ((screen.height() - 52.0) * 0.8 - 50.0).max(ROW_H * MIN_ROWS);
    let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, 52.0));
    let mut changed = false;
    let mut clicked = None;
    let modal = egui::Modal::new(id).area(area).backdrop_color(Color32::from_black_alpha(60)).show(ctx, |ui| {
        let pal = Palette::of(ui);
        let mark = ui.visuals().hyperlink_color;
        ui.set_width(width);
        let edit = TextEdit::singleline(&mut q.query)
            .hint_text(format!("{}  파일 이름으로 찾기", icon::SEARCH))
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 26.0))
            .vertical_align(egui::Align::Center);
        let r = ui.add(edit);
        if std::mem::take(&mut q.focus) {
            r.request_focus();
        }
        changed = r.changed();
        ui.add_space(4.0);

        let tree = match &app.tree {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                ui.label(RichText::new(format!("⚠ {e}")).small().color(pal.deleted));
                return;
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(11.0));
                    ui.label(RichText::new("파일 목록을 읽는 중…").small().color(pal.weak));
                });
                return;
            }
        };
        if q.hits.is_empty() {
            ui.label(RichText::new("맞는 파일이 없어요").small().color(pal.weak));
            return;
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        // 창(Area)은 지난 프레임 크기만큼만 자리를 주므로, 최소 높이도 같이 줘야 처음부터 길게 열린다.
        // (안 주면 ScrollArea 기본 최소 높이인 64px, 두 줄 반에서 멈춘다) 결과가 적으면 그만큼만 줄어든다.
        let scroll = ScrollArea::vertical().max_height(list_h).min_scrolled_height(list_h);
        scroll.auto_shrink([false, true]).show(ui, |ui| {
            let w = ui.available_width();
            for (i, hit) in q.hits.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
                if i == q.sel {
                    ui.painter().rect_filled(rect, CornerRadius::same(3), pal.selected);
                    if up || down {
                        resp.scroll_to_me(None);
                    }
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(3), pal.hover);
                }
                let node = &tree.nodes[hit.node];
                let job = row(node, &hit.marks, rect.width() - 20.0, mark, &pal);
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - g.size().y / 2.0), g, pal.text);
                if let Some(s) = node.status {
                    let color = crate::view::details::status_style(s, &pal).1;
                    let font = FontId::monospace(SMALL);
                    ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, s, font, color);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });

    if changed {
        q.sel = 0;
    }
    let open = clicked.or(enter.then_some(q.sel));
    let close = esc || modal.should_close();
    app.quick = Some(q);
    if changed {
        app.update_quick();
    }
    if let Some(i) = open {
        let hit = app.quick.take().and_then(|q| q.hits.into_iter().nth(i));
        let file = hit.zip(app.tree.as_ref().and_then(|t| t.as_ref().ok()));
        if let Some((hit, tree)) = file {
            let node = &tree.nodes[hit.node];
            let (path, status) = (node.path.clone(), node.status);
            app.open_file(&path, status, None);
        }
    } else if close {
        app.quick = None;
    }
}

/// 한 줄: 파일 이름, 그 뒤에 흐리게 폴더. 맞은 글자는 `mark` 색으로 쓴다.
fn row(node: &Node, marks: &[usize], width: f32, mark: Color32, pal: &Palette) -> LayoutJob {
    // `.gitignore`에 걸린 파일(.env 등)은 흐리게
    let color = if node.ignored { pal.weak } else { pal.text };
    let name = TextFormat { font_id: FontId::proportional(TEXT), color, ..Default::default() };
    let folder = TextFormat { font_id: FontId::proportional(LABEL), color: pal.weak, ..Default::default() };
    let mut job = LayoutJob::default();
    let at = node.path.len() - node.name().len();
    append(&mut job, &node.path, at..node.path.len(), marks, &name, mark);
    if at > 0 {
        job.append("", 10.0, folder.clone());
        append(&mut job, &node.path, 0..at - 1, marks, &folder, mark);
    }
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    job
}

/// `path[range]`를 붙인다. `marks`(맞은 글자 위치)는 `mark` 색으로 쓴다.
/// (배경 칠은 비례폭 글꼴에서 글자와 어긋나게 그려져서 쓰지 않는다)
fn append(job: &mut LayoutJob, path: &str, range: std::ops::Range<usize>, marks: &[usize], format: &TextFormat, mark: Color32) {
    let hit = TextFormat { color: mark, ..format.clone() };
    let mut start = range.start;
    let mut marked = false;
    for (i, _) in path[range.clone()].char_indices().map(|(i, c)| (i + range.start, c)) {
        let m = marks.binary_search(&i).is_ok();
        if m != marked {
            job.append(&path[start..i], 0.0, if marked { hit.clone() } else { format.clone() });
            (start, marked) = (i, m);
        }
    }
    job.append(&path[start..range.end], 0.0, if marked { hit } else { format.clone() });
}

//! 가운데 표: 그래프 | 설명(라벨 + 메시지) | 날짜 | 작성자 | 커밋
//! 커밋을 누르면 그 행 바로 아래에 상세가 펼쳐진다 (원본 Git Graph의 인라인 상세).
//! 더블클릭하면 그 커밋의 브랜치로 체크아웃하고, 오른쪽 클릭하면 작업 메뉴가 나온다.
//! 화면에 보이는 행만 그린다.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Id, Painter, PointerButton, Pos2, Rangef, Rect,
    RichText, ScrollArea, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};
use eframe::epaint::CubicBezierShape;
use eframe::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use std::sync::Arc;

use crate::app::{App, Loaded, RowClick, ScrollTo};
use crate::git::{RefKind, RefLabel, RowKind};
use crate::ops::Item;
use crate::style::{self, LABEL, Palette, SMALL, TEXT, graph_color};
use crate::view::{details, ops};

pub const ROW_H: f32 = 24.0;
const HEADER_H: f32 = 26.0;
const LANE_W: f32 = 16.0;
const GRAPH_PAD: f32 = 14.0;
const DOT_R: f32 = 4.0;
const LABEL_H: f32 = 17.0;

struct Columns {
    graph: Rangef,
    desc: Rangef,
    date: Option<Rangef>,
    author: Option<Rangef>,
    hash: Rangef,
}

impl Columns {
    fn new(rect: Rect, lanes: usize) -> Self {
        let lanes = lanes.max(1) as f32;
        let graph_w = (GRAPH_PAD * 2.0 + (lanes - 1.0) * LANE_W).clamp(44.0, (rect.width() * 0.35).max(44.0));
        let graph = Rangef::new(rect.left(), rect.left() + graph_w);
        let hash = Rangef::new(rect.right() - 76.0, rect.right());
        // 창이 좁으면 작성자, 날짜 순서로 숨긴다.
        let room = hash.min - graph.max;
        let author = (room > 560.0).then(|| Rangef::new(hash.min - 120.0, hash.min));
        let date_right = author.map_or(hash.min, |a| a.min);
        let date = (room > 400.0).then(|| Rangef::new(date_right - 130.0, date_right));
        let desc_right = date.map_or(date_right, |d| d.min);
        Self { graph, desc: Rangef::new(graph.max, desc_right), date, author, hash }
    }

    fn lane_x(&self, lane: usize) -> f32 {
        self.graph.min + GRAPH_PAD + lane as f32 * LANE_W
    }
}

/// 행 위치 계산. 펼쳐진 상세가 있으면 그 아래 행들은 상세 높이만큼 내려간다.
struct Rows {
    n: usize,
    open: Option<usize>,
    extra: f32,
}

impl Rows {
    fn top(&self, r: usize) -> f32 {
        let below = self.open.is_some_and(|s| r > s);
        r as f32 * ROW_H + if below { self.extra } else { 0.0 }
    }

    fn height(&self) -> f32 {
        self.n as f32 * ROW_H + if self.open.is_some() { self.extra } else { 0.0 }
    }

    /// 내용 좌표 y에 있는 행 (상세 영역이면 펼쳐진 행)
    fn at(&self, y: f32) -> usize {
        let plain = |y: f32| (y.max(0.0) / ROW_H) as usize;
        let r = match self.open {
            Some(s) if y >= (s + 1) as f32 * ROW_H => {
                let y = y - self.extra;
                if y < (s + 1) as f32 * ROW_H { s } else { plain(y) }
            }
            _ => plain(y),
        };
        r.min(self.n.saturating_sub(1))
    }

    /// 행(과 펼쳐진 상세)이 차지하는 높이
    fn span(&self, r: usize) -> f32 {
        ROW_H + if self.open == Some(r) { self.extra } else { 0.0 }
    }
}

enum Action {
    /// 누른 커밋, 행 번호, 그리고 라벨 위를 눌렀으면 그 라벨
    Select(String, usize, Option<RefLabel>),
    Request(Item),
    LoadMore,
    Details(details::Action),
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let pal = Palette::of(ui);
    if app.data.is_none() {
        empty_state(app, ui, &pal);
        return;
    }
    if let Some(err) = app.error.clone() {
        // 불러온 데이터가 있어도 오류는 위에 한 줄로 알려준다.
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new(format!("⚠ {err}")).small().color(pal.deleted));
        });
    }

    let scroll_to = app.scroll_to.take();
    // 보이는 높이는 이번 프레임 기준으로 (아래 diff가 열리고 닫히면 바로 바뀐다)
    let full = ui.available_rect_before_wrap();
    let offset = app.scroll_offset;
    let view_height = (full.height() - HEADER_H).max(ROW_H);
    let current_match = app.matches.get(app.match_pos).copied();
    let open_row = app.selected_row();
    let busy = app.busy();
    // 상세 높이는 화면을 넘지 않게
    let max_extra = (view_height - ROW_H * 2.0).max(140.0);
    let mut extra = app.settings.details_height.clamp(120.0, max_extra);
    let mut split = app.settings.details_split;
    let (extra_before, split_before) = (extra, split);
    let data = app.data.as_ref().unwrap();

    let cols = Columns::new(full, data.layout.lanes);
    header(ui, &cols, full, &pal);

    if data.snap.commits.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("아직 커밋이 없어요").color(pal.weak)));
        return;
    }

    let n = data.snap.commits.len() + usize::from(data.snap.more);
    let rows = Rows { n, open: open_row, extra };
    let mut area = ScrollArea::vertical().auto_shrink([false, false]).id_salt("commits");
    if let Some(target) = scroll_to {
        let new_offset = match target {
            ScrollTo::Center(r) => Some(rows.top(r) - view_height * 0.4),
            ScrollTo::Reveal(r) => {
                let (top, span) = (rows.top(r), rows.span(r));
                if top < offset {
                    Some(top)
                } else if top + span > offset + view_height {
                    Some((top + span - view_height).min(top))
                } else {
                    None
                }
            }
        };
        if let Some(o) = new_offset {
            area = area.vertical_scroll_offset(o.max(0.0));
        }
    }

    let mut action = None;
    let out = area.show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        ui.set_min_size(vec2(width, rows.height()));
        let origin = ui.max_rect().min;
        let painter = ui.painter().clone();
        let first = rows.at(viewport.min.y);
        let last = (rows.at(viewport.max.y) + 1).min(n);

        for row in first..last {
            let rect = Rect::from_min_size(origin + vec2(0.0, rows.top(row)), vec2(width, ROW_H));
            let Some(commit) = data.snap.commits.get(row) else {
                // 맨 아래 "더 불러오는 중" 행
                painter.text(
                    pos2(cols.desc.min + 8.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "더 불러오는 중…",
                    FontId::proportional(TEXT),
                    pal.weak,
                );
                action = Some(Action::LoadMore);
                continue;
            };
            let resp = ui.interact(rect, Id::new(("row", &commit.hash)), Sense::click());
            #[cfg(feature = "screenshot")]
            crate::devshot::row_rect(row, rect);
            let is_open = open_row == Some(row);
            let bg = if is_open {
                Some(pal.selected)
            } else if resp.hovered() || resp.context_menu_opened() {
                Some(pal.hover)
            } else {
                None
            };
            if let Some(bg) = bg {
                painter.rect_filled(rect, 0.0, bg);
            }
            if current_match == Some(row) {
                // 지금 보고 있는 검색 결과는 진하게 + 왼쪽 막대
                painter.rect_filled(rect, 0.0, pal.found.gamma_multiply(2.0));
                let bar = Rect::from_min_size(rect.left_top(), vec2(3.0, ROW_H));
                painter.rect_filled(bar, 0.0, Color32::from_rgb(0xff, 0xb3, 0x00));
            } else if !is_open && app.matches.binary_search(&row).is_ok() {
                painter.rect_filled(rect, 0.0, pal.found);
            }

            let on_label = paint_description(&painter, &cols, rect, data, row, resp.hover_pos(), &pal);
            if resp.clicked() {
                action = Some(Action::Select(commit.hash.clone(), row, on_label.cloned()));
            }
            resp.context_menu(|ui| {
                if let Some(item) = ops::row_menu(ui, &data.snap, row, busy) {
                    action = Some(Action::Request(item));
                }
            });
            let cy = rect.center().y;
            if let Some(date) = cols.date {
                cell(&painter, date, cy, &commit.date, FontId::proportional(TEXT), pal.weak);
            }
            if let Some(author) = cols.author {
                cell(&painter, author, cy, &commit.author, FontId::proportional(TEXT), pal.weak);
            }
            let is_real = !matches!(commit.kind, RowKind::Uncommitted { .. });
            let short = if is_real { &commit.hash[..commit.hash.len().min(7)] } else { "" };
            cell(&painter, cols.hash, cy, short, FontId::monospace(LABEL), pal.weak);
        }

        // 펼쳐진 상세
        if let Some(s) = open_row {
            let top = origin.y + rows.top(s) + ROW_H;
            let rect = Rect::from_x_y_ranges(cols.graph.max..=full.right(), top..=top + extra);
            if rect.intersects(ui.clip_rect()) {
                let hash = &data.snap.commits[s].hash;
                let open_file = app.diff.as_ref().map(|d| d.index);
                let mut state = details::Inline { split, height: extra, max_height: max_extra };
                if let Some(a) = details::inline(ui, rect, hash, &app.details, open_file, &mut state, &pal) {
                    action = Some(Action::Details(a));
                }
                split = state.split;
                extra = state.height;
            }
        }

        let clip = Rect::from_x_y_ranges(cols.graph, ui.clip_rect().y_range());
        let y = |r: usize| origin.y + rows.top(r) + ROW_H / 2.0;
        paint_graph(&painter.with_clip_rect(clip), &cols, data, first..last, &y, open_row, &pal);
    });

    app.scroll_offset = out.state.offset.y;
    // 화면이 좁아서 잠깐 줄어든 높이는 저장하지 않고, 직접 끌어서 바꾼 값만 저장한다.
    if extra != extra_before {
        app.settings.details_height = extra;
    }
    if split != split_before {
        app.settings.details_split = split;
    }

    let (clicked, double) = ui.input(|i| {
        (i.pointer.primary_clicked(), i.pointer.button_double_clicked(PointerButton::Primary))
    });
    // 더블클릭의 두 번째 클릭. 첫 클릭으로 상세가 펼쳐지면서 행이 밀렸을 수 있으니,
    // 지금 커서 아래가 아니라 첫 클릭이 누른 커밋으로 체크아웃한다. 이 클릭은 다른 일을 하지 않는다.
    if double {
        if let Some(click) = app.row_click.take() {
            app.double_click(click);
            return;
        }
    }
    if clicked {
        app.row_click = match &action {
            Some(Action::Select(hash, _, label)) => Some(RowClick { hash: hash.clone(), label: label.clone() }),
            _ => None,
        };
    }
    match action {
        Some(Action::Select(hash, row, _)) => {
            if app.selected.as_ref() == Some(&hash) {
                app.close_details();
            } else {
                app.select(hash);
                // 원본처럼 펼친 상세가 화면 안에 들어오게
                app.scroll_to = Some(ScrollTo::Reveal(row));
            }
        }
        Some(Action::Request(item)) => app.request(item),
        Some(Action::LoadMore) => app.load_more(),
        Some(Action::Details(a)) => details::apply(app, a),
        None => {}
    }
}

fn header(ui: &mut egui::Ui, cols: &Columns, full: Rect, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(full.width(), HEADER_H), Sense::hover());
    let p = ui.painter();
    let font = style::bold(LABEL);
    let cy = rect.center().y;
    let title = |range: Rangef, text: &str, pad: f32| {
        p.text(pos2(range.min + pad, cy), Align2::LEFT_CENTER, text, font.clone(), pal.weak);
    };
    title(cols.graph, "그래프", 10.0);
    title(cols.desc, "설명", 8.0);
    if let Some(d) = cols.date {
        title(d, "날짜", 8.0);
    }
    if let Some(a) = cols.author {
        title(a, "작성자", 8.0);
    }
    title(cols.hash, "커밋", 8.0);
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, pal.border));
}

fn cell(p: &Painter, range: Rangef, cy: f32, text: &str, font: FontId, color: Color32) {
    let g = truncated(p, text, font, color, range.span() - 16.0, false);
    p.galley(pos2(range.min + 8.0, cy - g.size().y / 2.0), g, color);
}

pub fn truncated(p: &Painter, text: &str, font: FontId, color: Color32, width: f32, italics: bool) -> Arc<Galley> {
    let format = TextFormat { font_id: font, color, italics, ..Default::default() };
    let mut job = LayoutJob::single_section(text.to_string(), format);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    p.layout_job(job)
}

/// 라벨과 커밋 메시지를 그린다. 커서(`pointer`)가 라벨 위에 있으면 그 라벨을 돌려준다.
fn paint_description<'a>(
    p: &Painter,
    cols: &Columns,
    rect: Rect,
    data: &'a Loaded,
    row: usize,
    pointer: Option<Pos2>,
    pal: &Palette,
) -> Option<&'a RefLabel> {
    let commit = &data.snap.commits[row];
    let color = graph_color(data.layout.nodes[row].color);
    let cy = rect.center().y;
    let right = cols.desc.max - 8.0;
    let mut x = cols.desc.min + 8.0;
    let mut hovered = None;
    if let Some(labels) = data.snap.refs.get(&commit.hash) {
        for label in labels {
            if x > right - 40.0 {
                break;
            }
            let end = paint_label(p, x, cy, label, color, pal);
            if pointer.is_some_and(|m| (x..end).contains(&m.x) && (m.y - cy).abs() <= LABEL_H / 2.0) {
                hovered = Some(label);
            }
            x = end + 5.0;
        }
    }
    let (text_color, italics) = match commit.kind {
        RowKind::Uncommitted { .. } => (pal.weak, true),
        // 원본처럼 머지 커밋은 흐리게
        _ if commit.parents.len() > 1 => (pal.weak, false),
        _ => (pal.text, false),
    };
    let g = truncated(p, &commit.subject, FontId::proportional(TEXT), text_color, right - x, italics);
    p.galley(pos2(x, cy - g.size().y / 2.0), g, text_color);
    hovered
}

/// 브랜치·태그 라벨을 그리고 오른쪽 끝 x를 돌려준다.
fn paint_label(p: &Painter, x: f32, cy: f32, label: &RefLabel, lane: Color32, pal: &Palette) -> f32 {
    let font = if label.is_head { style::bold(LABEL) } else { FontId::proportional(LABEL) };
    let neutral = if pal.bg.r() < 128 { Color32::from_gray(0x3a) } else { Color32::from_gray(0xe8) };
    let (fill, stroke, text, radius) = match label.kind {
        RefKind::Branch if label.is_head => (lane, lane, contrast_text(lane), 8),
        RefKind::Branch => (lane.gamma_multiply(0.22), lane, pal.text, 8),
        RefKind::Remote => (Color32::TRANSPARENT, lane.gamma_multiply(0.7), pal.weak, 8),
        RefKind::Tag => (neutral, pal.border, pal.text, 3),
        RefKind::Stash => (neutral, pal.border, pal.weak, 3),
    };
    let icon_w = if label.kind == RefKind::Tag { 11.0 } else { 0.0 };
    let name = p.layout_no_wrap(label.name.clone(), font, text);
    let remotes = (!label.remotes.is_empty()).then(|| {
        p.layout_no_wrap(label.remotes.join(" · "), FontId::proportional(SMALL), text.gamma_multiply(0.75))
    });
    let remote_w = remotes.as_ref().map_or(0.0, |g| g.size().x + 12.0);
    let w = 14.0 + icon_w + name.size().x + remote_w;
    let rect = Rect::from_min_size(pos2(x, cy - LABEL_H / 2.0), vec2(w, LABEL_H));
    let radius = CornerRadius::same(radius);
    p.rect_filled(rect, radius, fill);
    p.rect_stroke(rect, radius, Stroke::new(1.0, stroke), StrokeKind::Inside);

    let mut tx = x + 7.0;
    if label.kind == RefKind::Tag {
        // 작은 마름모 아이콘
        let c = pos2(tx + 3.5, cy);
        let pts = vec![c + vec2(0.0, -3.5), c + vec2(3.5, 0.0), c + vec2(0.0, 3.5), c + vec2(-3.5, 0.0)];
        p.add(Shape::convex_polygon(pts, pal.weak, Stroke::NONE));
        tx += icon_w;
    }
    let name_w = name.size().x;
    p.galley(pos2(tx, cy - name.size().y / 2.0), name, text);
    if let Some(g) = remotes {
        let sx = tx + name_w + 6.0;
        p.vline(sx, Rangef::new(rect.top() + 3.0, rect.bottom() - 3.0), Stroke::new(1.0, text.gamma_multiply(0.4)));
        p.galley(pos2(sx + 6.0, cy - g.size().y / 2.0), g, text);
    }
    rect.right()
}

fn contrast_text(bg: Color32) -> Color32 {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if lum > 150.0 { Color32::from_gray(0x1e) } else { Color32::WHITE }
}

fn paint_graph(
    p: &Painter,
    cols: &Columns,
    data: &Loaded,
    range: std::ops::Range<usize>,
    y: &dyn Fn(usize) -> f32,
    open_row: Option<usize>,
    pal: &Palette,
) {
    let layout = &data.layout;
    let commits = &data.snap.commits;
    let line = |color: Color32| Stroke::new(2.0, color);

    // 1) 선. 곡선은 배경색 그림자를 먼저 깔아서 겹치는 선과 구분되게 한다 (원본과 같은 방식).
    let first = range.start.saturating_sub(1);
    let last = range.end.min(layout.edges.len());
    let mut straight = Vec::new();
    let mut curves = Vec::new();
    for r in first..last {
        for e in &layout.edges[r] {
            let (xa, xb) = (cols.lane_x(e.from), cols.lane_x(e.to));
            let (ya, yb) = (y(r), y(r + 1));
            let color = if e.dashed { pal.uncommitted } else { graph_color(e.color) };
            if e.from == e.to {
                straight.push((pos2(xa, ya), pos2(xb, yb), color, e.dashed));
                continue;
            }
            if open_row != Some(r) {
                curves.push((pos2(xa, ya), pos2(xb, yb), color));
                continue;
            }
            // 펼쳐진 상세를 가로지르는 선: 곡선은 한 행 높이만 쓰고 나머지는 세로로 잇는다.
            // 다음 커밋으로 모이는 선은 아래쪽에서, 옆으로 갈라지는 선은 위쪽에서 휜다.
            let joins_next = layout.nodes.get(r + 1).is_some_and(|n| n.lane == e.to);
            if joins_next {
                straight.push((pos2(xa, ya), pos2(xa, yb - ROW_H), color, false));
                curves.push((pos2(xa, yb - ROW_H), pos2(xb, yb), color));
            } else {
                curves.push((pos2(xa, ya), pos2(xb, ya + ROW_H), color));
                straight.push((pos2(xb, ya + ROW_H), pos2(xb, yb), color, false));
            }
        }
    }
    for (a, b, color, dashed) in straight {
        if dashed {
            p.extend(Shape::dashed_line(&[a, b], line(color), 3.0, 3.0));
        } else {
            p.line_segment([a, b], line(color));
        }
    }
    for (a, b, color) in curves {
        let d = ROW_H * 0.8;
        let pts = [a, pos2(a.x, a.y + d), pos2(b.x, b.y - d), b];
        let shadow = Stroke::new(4.0, pal.bg.gamma_multiply(0.75));
        p.add(CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, shadow));
        p.add(CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, line(color)));
    }

    // 2) 점
    for r in range.start..range.end.min(commits.len()) {
        let node = layout.nodes[r];
        let c = Pos2::new(cols.lane_x(node.lane), y(r));
        let color = graph_color(node.color);
        let is_head = data.snap.head.as_ref() == Some(&commits[r].hash);
        match commits[r].kind {
            RowKind::Uncommitted { .. } => {
                p.circle(c, DOT_R, pal.bg, Stroke::new(1.5, pal.uncommitted));
            }
            RowKind::Stash { .. } => {
                p.circle(c, DOT_R + 0.5, color, Stroke::new(1.0, pal.bg.gamma_multiply(0.75)));
                p.circle_filled(c, 2.0, pal.bg);
            }
            RowKind::Commit if is_head => {
                p.circle(c, DOT_R, pal.bg, Stroke::new(2.0, color));
            }
            RowKind::Commit => {
                p.circle(c, DOT_R, color, Stroke::new(1.0, pal.bg.gamma_multiply(0.75)));
            }
        }
    }
}

fn empty_state(app: &mut App, ui: &mut egui::Ui, pal: &Palette) {
    ui.add_space((ui.available_height() * 0.28).max(20.0));
    ui.vertical_centered(|ui| {
        if app.loading {
            ui.label(RichText::new("불러오는 중…").color(pal.weak));
            return;
        }
        if let Some(err) = app.error.clone() {
            ui.label(RichText::new("저장소를 불러오지 못했어요").heading());
            ui.add_space(6.0);
            ui.label(RichText::new(err).small().color(pal.weak));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 190.0).max(0.0) / 2.0);
                if app.repo.is_some() && ui.button("다시 시도").clicked() {
                    app.reload();
                }
                if ui.button("다른 폴더 열기…").clicked() {
                    app.pick_folder();
                }
            });
            return;
        }
        ui.label(RichText::new("Git 저장소 폴더를 열어주세요").heading());
        ui.add_space(14.0);
        if ui.button("폴더 열기…  ⌘O").clicked() {
            app.pick_folder();
        }
        ui.add_space(18.0);
        ui.label(RichText::new("터미널에서는:  ggl-open <폴더>").small().color(pal.weak));
    });
}

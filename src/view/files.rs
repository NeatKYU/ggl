//! 왼쪽 파일 트리와, 트리에서 누른 파일의 내용(아래쪽 패널)
//! 트리에는 git이 아는 파일만 나온다 (`.gitignore`에 걸린 빌드 결과물 등은 빠진다).
//! 바뀐 파일은 색과 글자(M, U …)로, 바뀐 파일이 든 폴더는 점으로 표시한다.
//! 바뀐 파일을 열면 파일 전체에 바뀐 줄을 칠해서 보여준다.

use std::path::Path;
use std::process::Command;

use eframe::egui::{
    self, Align, Align2, CornerRadius, CursorIcon, FontId, Key, Layout, Margin, Rect, RichText, ScrollArea, Sense,
    Shape, Stroke, pos2, vec2,
};

use crate::app::App;
use crate::git::LineKind;
use crate::style::{self, LABEL, Palette, SMALL, TEXT};
use crate::tree::FileTree;
use crate::view::details::status_style;
use crate::view::diff;
use crate::view::table::truncated;

const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;
const MAX_MATCHES: usize = 500;

enum Action {
    Toggle(String),
    Open(String, Option<char>),
}

/// 한 줄에 그릴 것
struct Line<'a> {
    node: usize,
    label: &'a str,
    depth: usize,
    /// 검색 결과에서 이름 옆에 흐리게 붙이는 폴더
    folder: Option<&'a str>,
}

pub fn tree(app: &mut App, ui: &mut egui::Ui) {
    ui.set_min_size(ui.available_size());
    let pal = Palette::of(ui);

    egui::Frame::new().inner_margin(Margin { left: 10, right: 8, top: 8, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("파일").font(style::bold(TEXT)).color(pal.text));
            if let Some(Ok(t)) = &app.tree {
                let count = match t.changed {
                    0 => format!("{}개", t.files),
                    n => format!("{}개 · 바뀜 {n}", t.files),
                };
                ui.label(RichText::new(count).small().color(pal.weak));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let fold = ui.add_enabled(!app.tree_open.is_empty(), egui::Button::new(RichText::new("모두 접기").small()));
                if fold.clicked() {
                    app.tree_open.clear();
                }
            });
        });
        ui.add_space(2.0);
        let edit = egui::TextEdit::singleline(&mut app.tree_filter)
            .hint_text("파일 이름으로 찾기")
            .desired_width(f32::INFINITY);
        let r = ui.add(edit);
        if r.changed() {
            app.update_tree_matches();
        }
        if r.lost_focus() {
            let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if esc {
                app.tree_filter.clear();
                app.update_tree_matches();
            } else if enter {
                // Enter는 첫 번째 결과를 연다.
                let first = match (&app.tree, app.tree_matches.first()) {
                    (Some(Ok(t)), Some(&i)) => Some((t.nodes[i].path.clone(), t.nodes[i].status)),
                    _ => None,
                };
                if let Some((path, status)) = first {
                    app.open_file(&path, status);
                }
            }
        }
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().hline(sep.x_range(), sep.top(), Stroke::new(1.0, pal.border));

    let tree = match &app.tree {
        Some(Ok(t)) => t,
        other => {
            let text = match other {
                Some(Err(e)) => format!("파일 목록을 읽지 못했어요\n{e}"),
                _ => "불러오는 중…".to_string(),
            };
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(text).small().color(pal.weak)));
            return;
        }
    };

    let searching = !app.tree_filter.trim().is_empty();
    let rows = if searching { Vec::new() } else { tree.rows(&app.tree_open) };
    let lines: Vec<Line> = if searching {
        app.tree_matches
            .iter()
            .map(|&i| {
                let n = &tree.nodes[i];
                Line { node: i, label: n.name(), depth: 0, folder: Some(n.folder()) }
            })
            .collect()
    } else {
        rows.iter().map(|r| Line { node: r.node, label: tree.label(r), depth: r.depth, folder: None }).collect()
    };
    if lines.is_empty() {
        let text = if searching { "맞는 파일이 없어요" } else { "파일이 없어요" };
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(text).small().color(pal.weak)));
        return;
    }

    let repo = app.repo.clone().unwrap_or_default();
    let selected = app.file_view.as_ref().map(|v| v.path.as_str());
    let mut action = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    let n = lines.len() + usize::from(searching && lines.len() >= MAX_MATCHES);
    ScrollArea::vertical().id_salt("file-tree").auto_shrink([false, false]).show_rows(ui, ROW_H, n, |ui, range| {
        let width = ui.available_width();
        for i in range {
            let Some(line) = lines.get(i) else {
                let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
                let text = format!("… {MAX_MATCHES}개까지만 보여줘요");
                let font = FontId::proportional(SMALL);
                ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, font, pal.weak);
                continue;
            };
            let node = &tree.nodes[line.node];
            let open = node.dir && app.tree_open.contains(&node.path);
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            let bg = if !node.dir && selected == Some(node.path.as_str()) {
                Some(pal.selected)
            } else if resp.hovered() || resp.context_menu_opened() {
                Some(pal.hover)
            } else {
                None
            };
            if let Some(bg) = bg {
                ui.painter().rect_filled(rect.shrink2(vec2(4.0, 0.0)), CornerRadius::same(3), bg);
            }
            paint_line(ui.painter(), rect, tree, line, open, &pal);
            if resp.clicked() {
                action = Some(match node.dir {
                    true => Action::Toggle(node.path.clone()),
                    false => Action::Open(node.path.clone(), node.status),
                });
            }
            resp.context_menu(|ui| path_menu(ui, &repo, &node.path, node.dir));
            resp.on_hover_text(&node.path);
        }
    });

    match action {
        Some(Action::Toggle(path)) => {
            if !app.tree_open.remove(&path) {
                app.tree_open.insert(path);
            }
        }
        Some(Action::Open(path, status)) => app.open_file(&path, status),
        None => {}
    }
}

fn paint_line(p: &egui::Painter, rect: Rect, tree: &FileTree, line: &Line, open: bool, pal: &Palette) {
    let node = &tree.nodes[line.node];
    let cy = rect.center().y;
    let x = rect.left() + 10.0 + line.depth as f32 * INDENT;
    let mut right = rect.right() - 10.0;

    // 폴더는 펼침 삼각형, 파일은 그 자리를 비워서 이름 줄을 맞춘다.
    if node.dir {
        let c = pos2(x + 4.0, cy);
        let points = if open {
            vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
        } else {
            vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
        };
        p.add(Shape::convex_polygon(points, pal.weak, Stroke::NONE));
    }

    // 오른쪽 표시: 파일은 상태 글자, 바뀐 파일이 든 폴더는 점
    let mut name_color = pal.text;
    if let Some(s) = node.status {
        let (letter, color, _) = status_style(s, pal);
        let r = p.text(pos2(right, cy), Align2::RIGHT_CENTER, letter, FontId::monospace(LABEL), color);
        right = r.left() - 8.0;
        name_color = color;
    } else if node.dir && node.changed > 0 {
        let (_, color, _) = status_style('M', pal);
        p.circle_filled(pos2(right - 3.0, cy), 3.0, color);
        right -= 14.0;
    }

    let x = x + 14.0;
    let g = truncated(p, line.label, FontId::proportional(TEXT), name_color, right - x, false);
    let name_w = g.size().x;
    p.galley(pos2(x, cy - g.size().y / 2.0), g, name_color);
    if let Some(folder) = line.folder.filter(|f| !f.is_empty()) {
        let dx = x + name_w + 6.0;
        if right - dx > 24.0 {
            let g = truncated(p, folder, FontId::proportional(SMALL), pal.weak, right - dx, false);
            p.galley(pos2(dx, cy - g.size().y / 2.0), g, pal.weak);
        }
    }
}

/// 파일·폴더의 오른쪽 클릭 메뉴
fn path_menu(ui: &mut egui::Ui, repo: &Path, path: &str, dir: bool) {
    let full = repo.join(path);
    if ui.button("경로 복사").clicked() {
        ui.ctx().copy_text(path.to_string());
    }
    if ui.button("전체 경로 복사").clicked() {
        ui.ctx().copy_text(full.display().to_string());
    }
    ui.separator();
    if ui.button("Finder에서 보기").clicked() {
        let _ = Command::new("open").arg("-R").arg(&full).spawn();
    }
    if !dir && ui.button("기본 앱으로 열기").clicked() {
        let _ = Command::new("open").arg(&full).spawn();
    }
}

/// 아래쪽 패널: 트리에서 누른 파일
pub fn file(app: &mut App, ui: &mut egui::Ui) {
    ui.set_min_size(ui.available_size());
    let pal = Palette::of(ui);
    let Some(view) = &app.file_view else { return };

    let mut close = false;
    egui::Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("닫기").on_hover_text("Esc").clicked() {
                    close = true;
                }
                if let Some(Ok(d)) = &view.result {
                    let count = |k: LineKind| d.lines.iter().filter(|l| l.kind == k).count();
                    let (added, deleted) = (count(LineKind::Added), count(LineKind::Deleted));
                    if added + deleted > 0 {
                        ui.label(RichText::new(format!("−{deleted}")).small().color(pal.deleted));
                        ui.label(RichText::new(format!("+{added}")).small().color(pal.added));
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if let Some(s) = view.status {
                        let (letter, color, hint) = status_style(s, &pal);
                        ui.label(RichText::new(letter).monospace().color(color)).on_hover_text(hint);
                    }
                    let (dir, name) = view.path.rsplit_once('/').unwrap_or(("", &view.path));
                    let name = egui::Label::new(RichText::new(name).font(style::bold(TEXT)).color(pal.text)).truncate();
                    ui.add(name).on_hover_text(&view.path);
                    if !dir.is_empty() {
                        ui.add(egui::Label::new(RichText::new(dir).small().color(pal.weak)).truncate());
                    }
                });
            });
        });
    });
    let sep = ui.available_rect_before_wrap();
    ui.painter().hline(sep.x_range(), sep.top(), Stroke::new(1.0, pal.border));
    if close {
        app.file_view = None;
        return;
    }

    let message = |ui: &mut egui::Ui, text: &str, color| {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(text).color(color)));
    };
    match &view.result {
        None => message(ui, "불러오는 중…", pal.weak),
        Some(Err(e)) => message(ui, &format!("파일을 읽지 못했어요: {e}"), pal.deleted),
        Some(Ok(d)) if d.binary => message(ui, "바이너리 파일이라 내용을 보여줄 수 없어요", pal.weak),
        Some(Ok(d)) if d.lines.is_empty() && d.truncated => {
            message(ui, "파일이 너무 커서(2MB 초과) 보여주지 않아요", pal.weak)
        }
        Some(Ok(d)) if d.lines.is_empty() => message(ui, "빈 파일이에요", pal.weak),
        Some(Ok(d)) => diff::lines(ui, d, egui::Id::new(("file", &view.path)), &pal),
    }
}

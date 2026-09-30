//! 위쪽 도구 모음: 저장소 선택, 브랜치 필터, 리모트 표시, 검색, 새로고침, 리모트 새로고침

use std::path::PathBuf;

use eframe::egui::{self, Align, Key, Layout, RichText};

use crate::app::{App, FetchState, repo_name};
use crate::style::Palette;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        repo_picker(app, ui, &pal);
        branch_picker(app, ui);
        if ui.checkbox(&mut app.settings.show_remotes, "리모트 브랜치").changed() {
            app.toggle_remotes();
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let busy = app.loading && app.data.is_some();
            if ui.button("새로고침").on_hover_text("다시 불러오기 (⌘R)").clicked() {
                app.reload();
            }
            fetch_button(app, ui);

            match app.fetch_state.clone() {
                FetchState::Running => {
                    ui.label(RichText::new("리모트 가져오는 중…").small().color(pal.weak));
                    ui.add(egui::Spinner::new().size(12.0));
                }
                FetchState::Failed(err) => {
                    ui.label(RichText::new("⚠ 리모트 새로고침 실패").small().color(pal.deleted)).on_hover_text(err);
                }
                FetchState::Idle => {
                    let status = if busy {
                        "불러오는 중…".to_string()
                    } else if app.refreshed_at.is_empty() {
                        String::new()
                    } else {
                        format!("{} 갱신", app.refreshed_at)
                    };
                    ui.label(RichText::new(status).small().color(pal.weak))
                        .on_hover_text("파일이 바뀌면 자동으로 새로고침돼요");
                }
            }
            ui.add_space(8.0);
            search_box(app, ui, &pal);
        });
    });
}

/// 리모트 새로고침 (git fetch) 버튼
fn fetch_button(app: &mut App, ui: &mut egui::Ui) {
    let has_remotes = app.data.as_ref().is_some_and(|d| d.snap.has_remotes);
    let running = app.fetch_state == FetchState::Running;
    let r = ui.add_enabled(has_remotes && !running, egui::Button::new("리모트 새로고침"));
    let hint = if !has_remotes {
        "이 저장소에는 리모트가 없어요".to_string()
    } else {
        let last = if app.fetched_at.is_empty() {
            String::new()
        } else {
            format!("\n마지막으로 가져온 시각: {}", app.fetched_at)
        };
        format!("리모트의 새 커밋과 브랜치를 가져와요 (git fetch, ⌘⇧R)\n내 브랜치와 작업 파일은 바뀌지 않아요{last}")
    };
    let disabled_hint = if running { "리모트를 가져오는 중이에요" } else { "이 저장소에는 리모트가 없어요" };
    if r.on_hover_text(hint).on_disabled_hover_text(disabled_hint).clicked() {
        app.fetch();
    }
}

fn repo_picker(app: &mut App, ui: &mut egui::Ui, pal: &Palette) {
    let current = app.repo.as_deref().map_or_else(|| "저장소 없음".to_string(), repo_name);
    let mut open: Option<PathBuf> = None;
    let mut forget: Option<PathBuf> = None;
    let mut pick = false;
    egui::ComboBox::from_id_salt("repo")
        .selected_text(RichText::new(current).strong())
        .width(180.0)
        .height(400.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(320.0);
            for path in &app.settings.recent {
                let is_current = app.repo.as_ref() == Some(path);
                let r = ui
                    .horizontal(|ui| {
                        let r = ui.selectable_label(is_current, repo_name(path));
                        ui.label(RichText::new(path.display().to_string()).small().color(pal.weak));
                        r
                    })
                    .inner;
                if r.clicked() {
                    open = Some(path.clone());
                }
                r.context_menu(|ui| {
                    if ui.button("목록에서 지우기").clicked() {
                        forget = Some(path.clone());
                    }
                });
            }
            if !app.settings.recent.is_empty() {
                ui.separator();
            }
            if ui.button("폴더 열기…  ⌘O").clicked() {
                pick = true;
            }
        });
    if let Some(p) = open.filter(|p| app.repo.as_ref() != Some(p)) {
        app.open_repo(&p);
    }
    if let Some(p) = forget {
        app.forget_repo(&p);
    }
    if pick {
        app.pick_folder();
    }
}

fn branch_picker(app: &mut App, ui: &mut egui::Ui) {
    let branches = app.data.as_ref().map(|d| d.snap.branches.clone()).unwrap_or_default();
    let mut chosen = app.branch.clone();
    let label = chosen.as_deref().map_or("모든 브랜치", |b| b.strip_prefix("remotes/").unwrap_or(b));
    egui::ComboBox::from_id_salt("branch")
        .selected_text(label.to_string())
        .width(160.0)
        .height(420.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, None, "모든 브랜치");
            ui.separator();
            for b in &branches {
                let text = b.strip_prefix("remotes/").unwrap_or(b);
                ui.selectable_value(&mut chosen, Some(b.clone()), text);
            }
        });
    if chosen != app.branch {
        app.set_branch(chosen);
    }
}

fn search_box(app: &mut App, ui: &mut egui::Ui, pal: &Palette) {
    if !app.matches.is_empty() || !app.search.trim().is_empty() {
        let text = if app.matches.is_empty() {
            "결과 없음".to_string()
        } else {
            format!("{}/{}", app.match_pos + 1, app.matches.len())
        };
        ui.label(RichText::new(text).small().color(pal.weak));
    }
    let edit = egui::TextEdit::singleline(&mut app.search)
        .hint_text("검색  ⌘F")
        .desired_width(200.0);
    let r = ui.add(edit);
    if app.focus_search {
        r.request_focus();
        app.focus_search = false;
    }
    if r.changed() {
        app.update_matches();
    }
    // 한 줄 입력칸은 Enter를 누르면 포커스를 잃는다. 다음 결과로 이동하고 포커스는 되돌린다.
    if r.lost_focus() {
        let (enter, shift, esc) =
            ui.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift, i.key_pressed(Key::Escape)));
        if enter {
            app.step_match(!shift);
            r.request_focus();
        } else if esc {
            app.search.clear();
            app.update_matches();
        }
    }
}

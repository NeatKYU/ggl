//! 저장소를 바꾸는 작업의 화면: 커밋 오른쪽 클릭 메뉴, 실행 전 확인 창, 표 위의 알림

use eframe::egui::{self, Align, Id, Key, Layout, Modifiers, RichText, ScrollArea};

use crate::app::App;
use crate::git::{Snapshot, UNCOMMITTED};
use crate::ops::{self, Item, Op};
use crate::style::{self, Palette, TITLE};

/// 커밋 행의 오른쪽 클릭 메뉴. 고른 작업을 돌려준다.
pub fn row_menu(ui: &mut egui::Ui, snap: &Snapshot, row: usize, busy: bool) -> Option<Item> {
    let mut chosen = None;
    let groups = ops::menu(snap, row);
    let hash = snap.commits.get(row).map(|c| &c.hash).filter(|h| *h != UNCOMMITTED);
    let names = ops::branch_names(snap, row);
    let has_ops = !groups.is_empty();
    let has_copy = !names.is_empty() || hash.is_some();
    if !has_ops && !has_copy {
        ui.label(RichText::new("할 수 있는 작업이 없어요").weak());
    }
    for (i, group) in groups.into_iter().enumerate() {
        if i > 0 {
            ui.separator();
        }
        for item in group {
            let button = ui.add_enabled(!busy, egui::Button::new(&item.text));
            if button.on_disabled_hover_text("다른 작업이 끝난 뒤에 할 수 있어요").clicked() {
                chosen = Some(item);
            }
        }
    }
    // 복사: 이 커밋에 있는 브랜치 이름들, 그리고 커밋 해시
    if has_ops && has_copy {
        ui.separator();
    }
    for name in names {
        if ui.button(format!("브랜치 이름 복사: {name}")).clicked() {
            ui.ctx().copy_text(name);
        }
    }
    if let Some(hash) = hash {
        if ui.button("해시 복사").clicked() {
            ui.ctx().copy_text(hash.clone());
        }
    }
    chosen
}

pub fn has_banner(app: &App) -> bool {
    app.notice.is_some() || app.data.as_ref().is_some_and(|d| d.snap.in_progress.is_some())
}

/// 표 위의 알림: 충돌로 멈춘 작업(취소 버튼)과 실패한 작업의 git 메시지
pub fn banner(app: &mut App, ui: &mut egui::Ui) {
    let pal = Palette::of(ui);
    let mut close = false;
    if let Some(notice) = &app.notice {
        ui.horizontal(|ui| {
            let (title, color) = match notice.error {
                true => (format!("⚠ {}", notice.title), pal.deleted),
                false => (notice.title.clone(), pal.text),
            };
            ui.label(RichText::new(title).color(color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = ui.small_button("닫기").clicked();
            });
        });
        if !notice.body.is_empty() {
            // git 메시지는 길 수 있어서 높이를 제한하고 스크롤한다.
            ScrollArea::vertical().id_salt("notice").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
                let text = RichText::new(&notice.body).small().color(pal.weak);
                ui.add(egui::Label::new(text).selectable(true).wrap());
            });
        }
    }
    if close {
        app.notice = None;
    }
    if let Some(state) = app.data.as_ref().and_then(|d| d.snap.in_progress) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(ops::stalled(state)).color(pal.text));
            let item = ops::abort(state);
            if ui.add_enabled(!app.busy(), egui::Button::new(&item.text)).clicked() {
                app.request(item);
            }
        });
    }
}

/// 실행 전 확인 창. Enter로 실행, Esc나 바깥을 누르면 닫힌다.
pub fn confirm(app: &mut App, ctx: &egui::Context) {
    let Some(item) = &app.confirm else { return };
    let modal = egui::Modal::new(Id::new("confirm-op")).show(ctx, |ui| {
        let pal = Palette::of(ui);
        ui.set_width(380.0);
        ui.label(RichText::new(&item.text).font(style::bold(TITLE)).color(pal.text));
        ui.add_space(2.0);
        ui.label(RichText::new(item.op.command()).monospace().color(pal.weak));
        if let Some(warning) = warning(&item.op) {
            ui.add_space(6.0);
            ui.label(RichText::new(warning).small().color(pal.text));
        }
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let run = ui.button("실행").clicked();
            // "머지 취소" 창에서 헷갈리지 않게 '취소' 대신 '닫기'라고 쓴다.
            let cancel = ui.button("닫기").clicked();
            (run, cancel)
        })
        .inner
    });
    let (run, cancel) = modal.inner;
    let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if run || enter {
        if let Some(item) = app.confirm.take() {
            app.run_op(item.op);
        }
    } else if cancel || modal.should_close() {
        app.confirm = None;
    }
}

/// 실행하면 어떻게 되는지 한 줄로 알려준다.
fn warning(op: &Op) -> Option<&'static str> {
    match op {
        Op::Pull => Some("리모트의 새 커밋을 받아서 현재 브랜치에 합쳐요."),
        Op::Push { set_upstream: true, .. } => {
            Some("리모트에 올리고, 앞으로 이 브랜치가 그 리모트 브랜치를 따라가게 설정해요. 리모트에 없으면 새로 만들어요.")
        }
        Op::Push { .. } => Some("로컬 커밋을 리모트에 올려요. 리모트에 내가 받지 않은 커밋이 있으면 덮어쓰지 않고 거절돼요."),
        Op::Merge(_) => Some("충돌이 없으면 현재 브랜치에 바로 합쳐져요."),
        Op::CherryPick(_) => Some("이 커밋의 변경만 현재 브랜치에 새 커밋으로 복사해요."),
        Op::Abort(_) => Some("충돌을 해결하던 내용은 사라지고, 시작하기 전 상태로 돌아가요."),
        Op::Checkout(_) | Op::CheckoutRemote(_) => None,
    }
}

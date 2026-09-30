//! 개발용 스크린샷 (`--features screenshot`에서만 빌드됨)
//!
//! GGL_SHOT=out.png  [GGL_SELECT=행번호] [GGL_SEARCH=검색어] [GGL_THEME=light|dark] [GGL_WAIT=초] [GGL_DIFF=파일번호]
//! [GGL_FETCH=1] 리모트 새로고침을 누른 뒤 찍는다
//! [GGL_SCROLL=행수] 처음부터 그 행까지 스크롤하며 글자를 그려본다 (오래 켜둘 때 메모리 확인용)
//! 데이터(와 상세)가 다 불러와지면 화면을 저장하고 종료한다.

use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui;

use crate::app::App;

static STAGE: AtomicU32 = AtomicU32::new(0);

pub fn tick(app: &mut App, ui: &egui::Ui) {
    let Ok(path) = std::env::var("GGL_SHOT") else { return };
    let ctx = ui.ctx();
    ctx.request_repaint();

    let shot = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = shot {
        let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        let [w, h] = img.size;
        image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8).unwrap();
        let [aw, ah] = ctx.fonts(|f| f.font_image_size());
        eprintln!("font atlas: {aw}x{ah} ({:.1} MB)", (aw * ah * 4) as f64 / 1e6);
        std::process::exit(0);
    }

    let stage = STAGE.load(Ordering::Relaxed);
    match stage {
        0 if app.data.is_some() && !app.loading => {
            match std::env::var("GGL_THEME").as_deref() {
                Ok("light") => ctx.set_theme(egui::ThemePreference::Light),
                Ok("dark") => ctx.set_theme(egui::ThemePreference::Dark),
                _ => {}
            }
            if let Ok(q) = std::env::var("GGL_SEARCH") {
                app.search = q;
                app.update_matches();
            }
            if std::env::var("GGL_FETCH").is_ok() {
                app.fetch();
            }
            if let Some(row) = std::env::var("GGL_SELECT").ok().and_then(|s| s.parse().ok()) {
                app.select_row(row);
            }
            STAGE.store(1, Ordering::Relaxed);
        }
        1 if app.fetch_state == crate::app::FetchState::Running || app.loading => {}
        1 if (app.selected.is_none() || app.details.is_some()) && waited(ui) => {
            match std::env::var("GGL_DIFF").ok().and_then(|s| s.parse().ok()) {
                Some(i) if app.diff.is_none() => app.open_diff(i),
                _ if app.diff.as_ref().is_some_and(|d| d.result.is_none()) => {}
                _ => STAGE.store(2, Ordering::Relaxed),
            }
        }
        2 if !scrolled(app) => {}
        // 몇 프레임 더 그려서 레이아웃이 자리 잡게 한다.
        2..=5 => STAGE.store(stage + 1, Ordering::Relaxed),
        6 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            STAGE.store(7, Ordering::Relaxed);
        }
        _ => {}
    }
}

/// GGL_WAIT초가 지났는지 (그동안 저장소를 바꿔서 자동 새로고침을 확인할 때 쓴다)
fn waited(ui: &egui::Ui) -> bool {
    let wait: f64 = std::env::var("GGL_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    ui.input(|i| i.time) >= wait
}

/// GGL_SCROLL 행까지 한 화면씩 내려간다. 다 내려갔으면 true.
fn scrolled(app: &mut App) -> bool {
    static ROW: AtomicU32 = AtomicU32::new(0);
    let Some(target) = std::env::var("GGL_SCROLL").ok().and_then(|s| s.parse::<u32>().ok()) else {
        return true;
    };
    let loaded = app.data.as_ref().map_or(0, |d| d.snap.commits.len()) as u32;
    let row = ROW.load(Ordering::Relaxed);
    if row >= target.min(loaded) && (!app.data.as_ref().is_some_and(|d| d.snap.more) || loaded >= target) {
        return true;
    }
    let next = (row + 30).min(loaded.saturating_sub(1));
    ROW.store(next, Ordering::Relaxed);
    app.scroll_to = Some(crate::app::ScrollTo::Center(next as usize));
    if next + 40 >= loaded {
        app.load_more();
    }
    false
}

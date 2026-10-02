//! 폰트, 테마, 색상.

use std::borrow::Cow;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, Theme, ThemePreference,
};

/// 원본 Git Graph 기본 팔레트
pub const GRAPH_COLORS: [Color32; 12] = [
    Color32::from_rgb(0x00, 0x85, 0xd9),
    Color32::from_rgb(0xd9, 0x00, 0x8f),
    Color32::from_rgb(0x00, 0xd9, 0x0a),
    Color32::from_rgb(0xd9, 0x85, 0x00),
    Color32::from_rgb(0xa3, 0x00, 0xd9),
    Color32::from_rgb(0xff, 0x00, 0x00),
    Color32::from_rgb(0x00, 0xd9, 0xcc),
    Color32::from_rgb(0xe1, 0x38, 0xe8),
    Color32::from_rgb(0x85, 0xd9, 0x00),
    Color32::from_rgb(0xdc, 0x5b, 0x23),
    Color32::from_rgb(0x6f, 0x24, 0xd6),
    Color32::from_rgb(0xff, 0xcc, 0x00),
];

pub fn graph_color(i: usize) -> Color32 {
    GRAPH_COLORS[i % GRAPH_COLORS.len()]
}

pub const BOLD: &str = "bold";

// 글자 크기는 이 몇 가지만 쓴다. 크기마다 글자(특히 한글 음절)를 따로 그려서 캐시에 쌓기 때문에
// 종류가 적을수록 메모리를 덜 쓴다.
pub const SMALL: f32 = 11.0;
pub const LABEL: f32 = 12.0;
pub const TEXT: f32 = 13.0;
pub const TITLE: f32 = 15.0;

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// 버튼 아이콘 ([Phosphor](https://phosphoricons.com), MIT).
/// `assets/icons.ttf`는 아래 글자만 남긴 부분 폰트(약 5KB)라서, 아이콘을 늘리면 폰트도 다시 만들어야 한다:
/// `pyftsubset Phosphor.ttf --unicodes=<아래 코드 전부> --layout-features='' --output-file=assets/icons.ttf`
pub mod icon {
    pub const SIDEBAR: &str = "\u{ec24}";
    pub const FOLDER: &str = "\u{e25a}";
    pub const BRANCH: &str = "\u{e278}";
    pub const CLOUD: &str = "\u{e1aa}";
    pub const REFRESH: &str = "\u{e036}";
    pub const FETCH: &str = "\u{e1ac}";
    pub const PUSH: &str = "\u{e1ae}";
    pub const SEARCH: &str = "\u{e30c}";
    pub const TREE: &str = "\u{ee48}";
    pub const COLLAPSE: &str = "\u{e532}";
    pub const EDIT: &str = "\u{e3b4}";
    pub const SAVE: &str = "\u{e248}";
    pub const DONE: &str = "\u{e182}";
    pub const CLOSE: &str = "\u{e4f6}";
    pub const PREV: &str = "\u{e138}";
    pub const NEXT: &str = "\u{e13a}";
    pub const CASE: &str = "\u{e6ee}";
}

pub const ICON: f32 = 15.0;

/// 아이콘만 있는 버튼. 크기를 맞춰서 나란히 놓아도 가지런하다. 설명은 `on_hover_text`로 붙인다.
pub fn icon_button(icon: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(icon).size(ICON)).min_size(egui::vec2(26.0, 24.0))
}

/// 켜고 끄는 아이콘 버튼
pub fn icon_toggle(selected: bool, icon: &str) -> egui::Button<'static> {
    egui::Button::selectable(selected, egui::RichText::new(icon).size(ICON)).min_size(egui::vec2(26.0, 24.0))
}

/// 화면 곳곳에서 쓰는 색. 다크/라이트 모드에 따라 달라진다.
pub struct Palette {
    pub bg: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub found: Color32,
    pub border: Color32,
    pub added: Color32,
    pub deleted: Color32,
    pub uncommitted: Color32,
    /// 인라인 상세 배경 (원본의 rgba(128,128,128,0.1))
    pub inline_bg: Color32,
    pub diff_add_bg: Color32,
    pub diff_del_bg: Color32,
    pub diff_hunk_bg: Color32,
    pub diff_hunk: Color32,
}

impl Palette {
    pub fn of(ui: &egui::Ui) -> Self {
        let v = ui.visuals();
        if v.dark_mode {
            Self {
                bg: v.panel_fill,
                text: Color32::from_gray(0xcc),
                weak: Color32::from_gray(0x8c),
                hover: Color32::from_rgb(0x2a, 0x2d, 0x2e),
                selected: Color32::from_rgb(0x04, 0x39, 0x5e),
                found: Color32::from_rgba_unmultiplied(0xff, 0xcc, 0x00, 0x26),
                border: Color32::from_gray(0x3c),
                added: Color32::from_rgb(0x73, 0xc9, 0x91),
                deleted: Color32::from_rgb(0xf1, 0x4c, 0x4c),
                uncommitted: Color32::from_gray(0x80),
                inline_bg: Color32::from_gray(0x29),
                diff_add_bg: Color32::from_rgba_unmultiplied(0x2e, 0xa0, 0x43, 0x33),
                diff_del_bg: Color32::from_rgba_unmultiplied(0xf8, 0x51, 0x49, 0x33),
                diff_hunk_bg: Color32::from_rgba_unmultiplied(0x38, 0x8b, 0xfd, 0x1f),
                diff_hunk: Color32::from_rgb(0x79, 0xb8, 0xff),
            }
        } else {
            Self {
                bg: v.panel_fill,
                text: Color32::from_gray(0x33),
                weak: Color32::from_gray(0x80),
                hover: Color32::from_gray(0xf0),
                selected: Color32::from_rgb(0xcc, 0xe4, 0xf7),
                found: Color32::from_rgba_unmultiplied(0xff, 0xcc, 0x00, 0x40),
                border: Color32::from_gray(0xe0),
                added: Color32::from_rgb(0x10, 0x7c, 0x41),
                deleted: Color32::from_rgb(0xc7, 0x24, 0x24),
                uncommitted: Color32::from_gray(0x99),
                inline_bg: Color32::from_gray(0xf2),
                diff_add_bg: Color32::from_rgb(0xe6, 0xff, 0xec),
                diff_del_bg: Color32::from_rgb(0xff, 0xeb, 0xe9),
                diff_hunk_bg: Color32::from_rgb(0xdd, 0xf4, 0xff),
                diff_hunk: Color32::from_rgb(0x05, 0x50, 0xae),
            }
        }
    }
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);

    ctx.set_theme(ThemePreference::System);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |s| {
            use TextStyle::*;
            s.text_styles = [
                (Heading, bold(TITLE)),
                (Body, FontId::proportional(TEXT)),
                (Button, FontId::proportional(TEXT)),
                (Small, FontId::proportional(SMALL)),
                (Monospace, FontId::monospace(LABEL)),
            ]
            .into();
            s.spacing.button_padding = egui::vec2(8.0, 3.0);
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            let v = &mut s.visuals;
            if theme == Theme::Dark {
                // VS Code 다크 테마 느낌
                v.panel_fill = Color32::from_rgb(0x1e, 0x1e, 0x1e);
                v.window_fill = Color32::from_rgb(0x25, 0x25, 0x26);
                v.extreme_bg_color = Color32::from_rgb(0x31, 0x31, 0x31);
                v.faint_bg_color = Color32::from_rgb(0x25, 0x25, 0x26);
            } else {
                v.panel_fill = Color32::WHITE;
                v.window_fill = Color32::from_gray(0xf8);
                v.faint_bg_color = Color32::from_gray(0xf5);
                v.extreme_bg_color = Color32::from_gray(0xf3);
            }
        });
    }
}

/// macOS 시스템 폰트를 메모리 매핑으로 불러온다 (앱 크기·메모리가 늘지 않음).
/// egui 기본 폰트에는 한글이 없어서 꼭 필요하다.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, path: &str, index: u32| -> bool {
        let Some(bytes) = map_file(path) else { return false };
        let data = FontData { font: Cow::Borrowed(bytes), index, tweak: Default::default() };
        fonts.font_data.insert(name.to_string(), Arc::new(data));
        true
    };
    let sf = add("sf", "/System/Library/Fonts/SFNS.ttf", 0);
    let sf_mono = add("sf-mono", "/System/Library/Fonts/SFNSMono.ttf", 0);
    let kr = add("kr", "/System/Library/Fonts/AppleSDGothicNeo.ttc", 0);
    let kr_bold = add("kr-bold", "/System/Library/Fonts/AppleSDGothicNeo.ttc", 6);

    let defaults = fonts.families[&FontFamily::Proportional].clone();
    let mono_defaults = fonts.families[&FontFamily::Monospace].clone();
    // 아이콘은 앱에 넣어둔 작은 폰트. 사용자 영역(PUA) 글자라서 어느 글꼴 뒤에 붙여도 겹치지 않는다.
    let icons = FontData::from_static(include_bytes!("../assets/icons.ttf"));
    fonts.font_data.insert("icons".to_string(), Arc::new(icons));
    let pick = |list: &[(&str, bool)], rest: &[String]| -> Vec<String> {
        let mut v: Vec<String> = list.iter().filter(|(_, ok)| *ok).map(|(n, _)| n.to_string()).collect();
        v.extend(rest.iter().cloned());
        v.push("icons".to_string());
        v
    };
    fonts.families.insert(FontFamily::Proportional, pick(&[("sf", sf), ("kr", kr)], &defaults));
    fonts.families.insert(FontFamily::Monospace, pick(&[("sf-mono", sf_mono), ("kr", kr)], &mono_defaults));
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), pick(&[("kr-bold", kr_bold), ("sf", sf)], &defaults));
    ctx.set_fonts(fonts);
}

fn map_file(path: &str) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: 시스템 폰트 파일은 실행 중에 바뀌지 않는다.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let map: &'static memmap2::Mmap = Box::leak(Box::new(map));
    Some(&map[..])
}

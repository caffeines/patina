//! Icons, drawn from the exact SVG path data in the design (16×16 viewBox).

use std::hash::{Hash, Hasher};

use egui::{Color32, Rect, Ui};

pub const DASHBOARD: &str = "M2.6 2.6h4.2v5.2H2.6zM9.2 2.6h4.2v3H9.2zM9.2 7.8h4.2v5.6H9.2zM2.6 10.2h4.2v3.2H2.6z";
pub const CONTAINERS: &str = "M8 1.8l5.5 3.1v6.2L8 14.2l-5.5-3.1V4.9zM2.5 4.9L8 8l5.5-3.1M8 8v6.2";
pub const IMAGES: &str = "M8 2l6 3.2-6 3.2-6-3.2zM2 8l6 3.2L14 8M2 10.8L8 14l6-3.2";
pub const VOLUMES: &str = "M3 4.2c0-1.2 2.2-2.2 5-2.2s5 1 5 2.2-2.2 2.2-5 2.2-5-1-5-2.2zM3 4.2v7.6c0 1.2 2.2 2.2 5 2.2s5-1 5-2.2V4.2M3 8c0 1.2 2.2 2.2 5 2.2s5-1 5-2.2";
pub const NETWORKS: &str = "M6.4 3.6a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M2 12.2a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M10.8 12.2a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M8 5.2v2.6M8 7.8l-3.6 3M8 7.8l3.6 3";
pub const MACHINES: &str = "M4 2.8h8a2.2 2.2 0 0 1 2.2 2.2v6a2.2 2.2 0 0 1-2.2 2.2H4A2.2 2.2 0 0 1 1.8 11V5A2.2 2.2 0 0 1 4 2.8zM4.8 6.4L7 8 4.8 9.6M8.6 9.8h2.6";
pub const KUBERNETES: &str = "M8 1.6l5.6 3.2v6.4L8 14.4l-5.6-3.2V4.8zM6.2 8a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0-3.6 0";
pub const BUILDS: &str = "M8.6 2.4l1.6-.9 4.3 4.3-.9 1.6zM9.6 4.9l1.5 1.5M10.4 6.6l-6.5 6.5a1.1 1.1 0 0 1-1.6-1.6l6.5-6.5";
pub const SETTINGS: &str = "M2.5 5h6M11.5 5h2M8.5 5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0M2.5 11h2M7.5 11h6M4.5 11a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0";

pub const PLUS: &str = "M8 3.5v9M3.5 8h9";
pub const CLOSE: &str = "M4.5 4.5l7 7M11.5 4.5l-7 7";
pub const SEARCH: &str = "M7 2.8a4.2 4.2 0 1 0 0 8.4a4.2 4.2 0 0 0 0-8.4zM10.2 10.2l3.3 3.3";
pub const PLUG: &str = "M8 2.2v5.3M4.6 4.4a5 5 0 1 0 6.8 0";
pub const STOP: &str = "M4.5 4.5h7v7h-7z";
pub const PLAY: &str = "M5.5 3.5v9l7-4.5z";
pub const MORE: &str = "M3.2 6.8a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 0 0 0-2.4zM8 6.8a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 0 0 0-2.4zM12.8 6.8a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 0 0 0-2.4z";
pub const RESTART: &str = "M12.9 8a4.9 4.9 0 1 1-1.5-3.5M12.9 2.4v2.9H10";
pub const BACK: &str = "M9.8 3.5L5.3 8l4.5 4.5";
pub const DOWN: &str = "M8 3v9.5M4 8.5l4 4 4-4";
pub const SCALE: &str = "M8 2.5v11M5.2 5.3L8 2.5l2.8 2.8M5.2 10.7L8 13.5l2.8-2.8";
pub const EXPAND: &str = "M9.5 2.5h4v4M13.5 2.5L9 7M6.5 13.5h-4v-4M2.5 13.5L7 9";
pub const INFO: &str = "M8 2.2a5.8 5.8 0 1 0 0 11.6A5.8 5.8 0 0 0 8 2.2zM8 7.2v4M8 4.9v.1";
pub const TRASH: &str = "M3.5 4.5h9M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5M6.8 7v3.5M9.2 7v3.5";

pub fn nav(view: crate::sim::View) -> &'static str {
    use crate::sim::View::*;
    match view {
        Dashboard => DASHBOARD,
        Containers => CONTAINERS,
        Images => IMAGES,
        Volumes => VOLUMES,
        Networks => NETWORKS,
        Builds => BUILDS,
        Machines => MACHINES,
        Kubernetes => KUBERNETES,
        Settings => SETTINGS,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Stroke(f32),
    Fill,
}

/// Paint an icon into `rect`, tinted with `color`. The SVG is rasterized
/// once per size and cached by egui.
pub fn paint(ui: &Ui, rect: Rect, d: &str, ink: Ink, color: Color32) {
    let style = match ink {
        Ink::Stroke(w) => {
            format!(r##"fill="none" stroke="#ffffff" stroke-width="{w}" stroke-linecap="round" stroke-linejoin="round""##)
        }
        Ink::Fill => r##"fill="#ffffff""##.to_string(),
    };
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="{d}" {style}/></svg>"##);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut h);
    let uri = format!("bytes://patina-icon-{:016x}.svg", h.finish());
    egui::Image::from_bytes(uri, svg.into_bytes()).tint(color).fit_to_exact_size(rect.size()).paint_at(ui, rect);
}

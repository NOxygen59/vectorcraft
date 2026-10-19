//! The header's resource readout (iPad): the memory and CPU VectorCraft itself uses, the battery
//! level, and a 60 / 120 Hz switch, left of the Discord button. Shown only when the platform
//! provides the readings. They are cached and refreshed every two seconds, so the readout costs
//! one small redraw per period and nothing between.

use std::time::{Duration, Instant};

use egui::{CornerRadius, Ui};

use crate::theme::Tokens;
use crate::{ResourceReading, VectorcraftApp};

/// How often the readout is refreshed.
const PERIOD: Duration = Duration::from_secs(2);

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens) {
    let Some(read) = app.services.resources else { return };
    let ctx = ui.ctx().clone();
    let cache_id = egui::Id::new("vectorcraft.resource_readout");
    let fps_id = egui::Id::new("vectorcraft.max_frame_rate");
    let now = Instant::now();
    let cached: Option<(Instant, ResourceReading)> = ctx.data(|d| d.get_temp(cache_id));
    let reading = match cached {
        Some((at, r)) if now.duration_since(at) < PERIOD => r,
        _ => {
            let r = read();
            ctx.data_mut(|d| d.insert_temp(cache_id, (now, r)));
            r
        }
    };
    ctx.request_repaint_after(PERIOD);

    // The frame-rate cap, remembered between launches; applied once at startup.
    let mut fps: u32 = ctx.data_mut(|d| d.get_persisted(fps_id)).unwrap_or(60);
    if let Some(set) = app.services.set_max_frame_rate {
        let applied_id = egui::Id::new("vectorcraft.max_frame_rate.applied");
        if !ctx.data(|d| d.get_temp::<bool>(applied_id).unwrap_or(false)) {
            set(fps);
            ctx.data_mut(|d| d.insert_temp(applied_id, true));
        }
        if ui.available_width() < 70.0 {
            return;
        }
        ui.add_space(10.0);
        let label = egui::RichText::new(format!("{fps} Hz")).color(if fps > 60 { t.accent } else { t.text_dim }).size(12.0).monospace();
        let tip = tl!("Maximum frame rate: 60 Hz saves battery and heat, 120 Hz is smoother. Tap to switch.");
        let button = egui::Button::new(label)
            .fill(t.hover)
            .corner_radius(CornerRadius::same(4))
            .min_size(egui::vec2(52.0, 20.0));
        if ui.add(button).on_hover_text(tip).clicked() {
            fps = if fps > 60 { 60 } else { 120 };
            set(fps);
            ctx.data_mut(|d| d.insert_persisted(fps_id, fps));
        }
    }

    let decimal = |v: f64| {
        let s = format!("{v:.1}");
        // Languages that write 2,5 rather than 2.5.
        let comma = matches!(
            crate::i18n::current().code(),
            "fr" | "de" | "es" | "it" | "pt-br" | "ru" | "uk" | "cs" | "pl" | "nl" | "tr" | "id" | "vi" | "el" | "hu" | "sr"
        );
        if comma { s.replace('.', ",") } else { s }
    };
    let mib = reading.memory_bytes as f64 / (1024.0 * 1024.0);
    let memory = if mib >= 1024.0 {
        tl!("{n} GB").replace("{n}", &decimal(mib / 1024.0))
    } else {
        tl!("{n} MB").replace("{n}", &format!("{mib:.0}"))
    };
    let base = format!("{} {memory}  ·  {} {:.0} %", tl!("RAM"), tl!("CPU"), reading.cpu_percent);
    let mut full = base.clone();
    if let Some(level) = reading.battery_percent {
        full.push_str(&format!("  ·  {} {level:.0} %", tl!("Battery")));
        if reading.charging {
            full.push_str(" +");
        }
    }
    // A narrow window (Stage Manager, Aerial) drops the battery, then the whole readout.
    let font = egui::FontId::monospace(12.0);
    let width = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), t.text_dim).size().x + 8.0;
    let room = ui.available_width();
    let text = if width(&full) <= room {
        full
    } else if width(&base) <= room {
        base
    } else {
        return;
    };
    let tip = tl!("Memory and processor used by VectorCraft itself, and the battery level. Updated every 2 seconds.");
    ui.add_space(8.0);
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim).size(12.0).monospace()).selectable(false)).on_hover_text(tip);
}

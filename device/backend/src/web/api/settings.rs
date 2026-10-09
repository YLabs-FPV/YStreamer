use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use crate::settings::{Settings, validate_hostname, validate_timezone};
use crate::system;

use super::display::overlay_changed;
use super::{ApiError, Services, bad, failed};

#[derive(Serialize)]
pub(super) struct Saved {
    settings: Settings,
    /// Something the user should know, e.g. that WiFi is reconnecting
    notice: Option<String>,
}

pub(super) async fn get_settings(State(sv): State<Services>) -> Json<Settings> {
    Json(sv.store.get())
}

pub(super) async fn put_all(
    State(sv): State<Services>,
    Json(next): Json<Settings>,
) -> Result<Json<Saved>, ApiError> {
    commit(&sv, next).await
}

pub(super) async fn reset(State(sv): State<Services>) -> Result<Json<Saved>, ApiError> {
    commit(&sv, Settings::default()).await
}

pub(super) async fn put_section(
    State(sv): State<Services>,
    Path(section): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Saved>, ApiError> {
    // Splice the section into the current settings as JSON and parse the
    // whole thing back, so every section shares one code path
    let mut all = serde_json::to_value(sv.store.get()).expect("settings serialize");
    match all.get_mut(&section) {
        Some(slot) => *slot = body,
        None => {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("No settings section '{section}'"),
            ));
        }
    }
    let next: Settings =
        serde_json::from_value(all).map_err(|e| bad(format!("Invalid {section} settings: {e}")))?;
    commit(&sv, next).await
}

pub(super) fn validate(s: &Settings) -> Result<(), String> {
    validate_hostname(&s.device.hostname)?;
    validate_timezone(&s.device.timezone)?;
    s.input.uvc.validate()?;
    s.overlay.validate()?;
    s.wifi.validate()?;
    s.ethernet.validate()?;
    s.rtsp.validate()?;
    s.srt.validate()?;
    s.rtmp.validate()?;
    s.udp.validate()?;
    s.recording.validate()?;
    Ok(())
}

pub(super) async fn commit(sv: &Services, next: Settings) -> Result<Json<Saved>, ApiError> {
    // Also covers reset and import, which bring "System" modes along
    let next = next.for_install();
    validate(&next).map_err(bad)?;
    let prev = sv.store.get();
    let mut notice = None;

    if next.rtsp != prev.rtsp {
        sv.rtsp.apply(&next.rtsp).map_err(|e| {
            let _ = sv.rtsp.apply(&prev.rtsp);
            failed(format!("RTSP server: {e}"))
        })?;
    }

    if next.srt != prev.srt
        && let Err(e) = sv.srt.apply(&next.srt)
    {
        if next.rtsp != prev.rtsp {
            let _ = sv.rtsp.apply(&prev.rtsp);
        }
        let _ = sv.srt.apply(&prev.srt);
        return Err(failed(format!("SRT: {e}")));
    }

    if next.udp != prev.udp
        && let Err(e) = sv.udp.apply(&next.udp)
    {
        if next.rtsp != prev.rtsp {
            let _ = sv.rtsp.apply(&prev.rtsp);
        }
        if next.srt != prev.srt {
            let _ = sv.srt.apply(&prev.srt);
        }
        let _ = sv.udp.apply(&prev.udp);
        return Err(failed(format!("UDP: {e}")));
    }

    if next.input != prev.input {
        sv.input.set_mode(next.input.mode);
        sv.uvc.apply(&next.input.uvc);
    }

    // Never refused: an unreachable server is retried in the background
    if next.rtmp != prev.rtmp {
        sv.rtmp.apply(&next.rtmp);
    }

    let display_changed = next.display != prev.display;
    let rollback = |sv: &Services| {
        if next.rtsp != prev.rtsp {
            let _ = sv.rtsp.apply(&prev.rtsp);
        }
        if next.srt != prev.srt {
            let _ = sv.srt.apply(&prev.srt);
        }
        if next.rtmp != prev.rtmp {
            sv.rtmp.apply(&prev.rtmp);
        }
        if next.udp != prev.udp {
            let _ = sv.udp.apply(&prev.udp);
        }
        if next.input != prev.input {
            sv.input.set_mode(prev.input.mode);
            sv.uvc.apply(&prev.input.uvc);
        }
        if display_changed {
            let _ = sv.player.apply(&prev.display);
        }
    };

    if display_changed && let Err(e) = sv.player.apply(&next.display) {
        rollback(sv);
        return Err(failed(format!("HDMI output: {e}")));
    }

    let timezone_changed = next.device.timezone != prev.device.timezone;
    if timezone_changed && let Err(e) = system::set_timezone(&next.device.timezone).await {
        rollback(sv);
        return Err(failed(format!("Couldn't set the timezone: {e}")));
    }

    if next.device.hostname != prev.device.hostname {
        if let Err(e) = system::set_hostname(&next.device.hostname).await {
            rollback(sv);
            if timezone_changed {
                let _ = system::set_timezone(&prev.device.timezone).await;
            }
            return Err(failed(format!("Couldn't rename the device: {e}")));
        }
        notice = Some(format!("Now reachable as {}.local", next.device.hostname));
    }

    let saved = sv.store.update(|s| *s = next.clone()).map_err(|e| {
        rollback(sv);
        failed(e)
    })?;

    if saved.recording != prev.recording {
        sv.recorder.configure(&saved.recording);
        if saved.recording.button != prev.recording.button {
            sv.button.configure(&saved.recording.button);
        }
    }

    if saved.overlay != prev.overlay {
        sv.overlay.set_cfg(&saved.overlay);
        overlay_changed(sv);
    }

    if saved.wifi != prev.wifi {
        sv.network.apply_later(saved.wifi.clone());
        notice = Some(
            "Applying WiFi settings. If you're connected over WiFi, you may need to reconnect."
                .into(),
        );
    }

    if saved.ethernet != prev.ethernet {
        sv.network.apply_ethernet_later(saved.ethernet.clone());
        notice = Some(match saved.ethernet.mode {
            crate::settings::EthernetMode::Static => format!(
                "Applying Ethernet settings. Over Ethernet, this page moves to http://{}",
                saved.ethernet.address.split('/').next().unwrap_or_default()
            ),
            crate::settings::EthernetMode::Auto => {
                "Applying Ethernet settings. If you're connected over Ethernet, the address may change."
                    .into()
            }
            crate::settings::EthernetMode::System => {
                "Handing Ethernet back to the system's own settings.".into()
            }
        });
    }

    Ok(Json(Saved {
        settings: saved,
        notice,
    }))
}

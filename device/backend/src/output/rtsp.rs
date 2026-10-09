use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use gstreamer_rtsp_server::prelude::*;
use gstreamer_rtsp_server::{
    RTSP_PERM_MEDIA_FACTORY_ACCESS, RTSP_PERM_MEDIA_FACTORY_CONSTRUCT,
    RTSP_TOKEN_MEDIA_FACTORY_ROLE, RTSPAuth, RTSPFilterResult, RTSPMediaFactory, RTSPServer,
    RTSPToken, glib, gst_rtsp,
};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::settings::RtspSettings;

/// Role granted to authenticated users when auth is on
const ROLE: &str = "viewer";

/// A running server instance; dropped (and detached) on reconfigure
struct Running {
    server: RTSPServer,
    source: glib::SourceId,
}

pub struct RtspServer {
    appsrc: Arc<Mutex<Option<AppSrc>>>,
    running: Mutex<Option<Running>>,
}

impl RtspServer {
    pub fn new(cfg: &RtspSettings) -> anyhow::Result<Self> {
        gst::init()?;

        thread::spawn(move || {
            let main_loop = glib::MainLoop::new(None, false);
            main_loop.run();
        });

        let this = Self {
            appsrc: Arc::new(Mutex::new(None)),
            running: Mutex::new(None),
        };
        if let Err(e) = this.apply(cfg) {
            eprintln!("[rtsp] not started: {e}");
        }
        Ok(this)
    }

    pub fn apply(&self, cfg: &RtspSettings) -> anyhow::Result<()> {
        let mut running = self.running.lock().unwrap();
        if let Some(old) = running.take() {
            old.server
                .client_filter(Some(&mut |_, _| RTSPFilterResult::Remove));
            old.source.remove();
            *self.appsrc.lock().unwrap() = None;
            eprintln!("[rtsp] stopped");
        }
        if !cfg.enabled {
            return Ok(());
        }

        let server = RTSPServer::new();
        server.set_service(&cfg.port.to_string());

        let mounts = server
            .mount_points()
            .ok_or_else(|| anyhow::anyhow!("RTSP server has no mount points"))?;

        let factory = RTSPMediaFactory::new();
        factory.set_launch(
            "( appsrc name=src is-live=true format=time do-timestamp=true ! \
             h264parse config-interval=-1 ! \
             rtph264pay name=pay0 pt=96 )",
        );
        factory.set_shared(true);
        if cfg.tcp_only {
            factory.set_protocols(gst_rtsp::RTSPLowerTrans::TCP);
        }

        if cfg.auth {
            let auth = RTSPAuth::new();
            let token = RTSPToken::new(&[(RTSP_TOKEN_MEDIA_FACTORY_ROLE, &ROLE)]);
            auth.add_basic(&RTSPAuth::make_basic(&cfg.username, &cfg.password), &token);
            server.set_auth(Some(&auth));
            factory.add_role_from_structure(
                &gst::Structure::builder(ROLE)
                    .field(RTSP_PERM_MEDIA_FACTORY_ACCESS, true)
                    .field(RTSP_PERM_MEDIA_FACTORY_CONSTRUCT, true)
                    .build(),
            );
        }

        let appsrc_slot_cb = Arc::clone(&self.appsrc);
        factory.connect_media_configure(move |_factory, media| {
            let element = media.element();
            let bin = element
                .dynamic_cast_ref::<gst::Bin>()
                .expect("RTSP media element is a bin");
            let src = bin
                .by_name("src")
                .expect("no appsrc named 'src' in RTSP pipeline")
                .dynamic_cast::<AppSrc>()
                .expect("'src' element is not an appsrc");

            let caps = gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .field("alignment", "stream")
                .build();
            src.set_caps(Some(&caps));
            src.set_stream_type(gstreamer_app::AppStreamType::Stream);
            src.set_is_live(true);
            src.set_format(gst::Format::Time);

            *appsrc_slot_cb.lock().unwrap() = Some(src);
        });

        mounts.add_factory(&cfg.path, factory);
        let source = server
            .attach(None)
            .map_err(|_| anyhow::anyhow!("couldn't listen on port {}", cfg.port))?;

        eprintln!(
            "[rtsp] serving at rtsp://<this-host>:{}{}",
            cfg.port, cfg.path
        );
        *running = Some(Running { server, source });
        Ok(())
    }

    /// Connected players, or None while the server is off
    pub fn clients(&self) -> Option<usize> {
        let running = self.running.lock().unwrap();
        Some(running.as_ref()?.server.client_filter(None).len())
    }

    pub fn push(&self, data: &[u8]) {
        let guard = self.appsrc.lock().unwrap();
        let Some(src) = guard.as_ref() else {
            return;
        };
        let Ok(mut buffer) = gst::Buffer::with_size(data.len()) else {
            return;
        };
        {
            let buffer_ref = buffer.get_mut().unwrap();
            if buffer_ref.copy_from_slice(0, data).is_err() {
                return;
            }
        }
        let _ = src.push_buffer(buffer);
    }
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpiocdev::Request;
use gpiocdev::line::{Bias, EdgeDetection, EdgeKind, Value};
use serde::Serialize;

use crate::output::recording::Recorder;
use crate::settings::{ButtonKind, ButtonSettings};

pub(super) const CHIP: &str = "/dev/gpiochip0";
const DEBOUNCE: Duration = Duration::from_millis(50);
const POLL: Duration = Duration::from_millis(200);
const RETRY: Duration = Duration::from_secs(5);

#[derive(Serialize, Clone, Default)]
pub struct ButtonStatus {
    pub ready: bool,
    pub error: Option<String>,
    pub presses: u64,
}

pub struct RecordButton {
    cfg: Mutex<ButtonSettings>,
    /// Bumped on every settings change, so the thread reopens its pins
    generation: AtomicU64,
    status: Mutex<ButtonStatus>,
    recorder: Arc<Recorder>,
}

impl RecordButton {
    pub fn start(cfg: &ButtonSettings, recorder: Arc<Recorder>) -> Arc<Self> {
        let this = Arc::new(Self {
            cfg: Mutex::new(cfg.clone()),
            generation: AtomicU64::new(0),
            status: Mutex::new(ButtonStatus::default()),
            recorder,
        });
        let worker = Arc::clone(&this);
        thread::spawn(move || worker.run());
        this
    }

    pub fn configure(&self, cfg: &ButtonSettings) {
        *self.cfg.lock().unwrap() = cfg.clone();
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn status(&self) -> ButtonStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_state(&self, ready: bool, error: Option<String>) {
        let mut status = self.status.lock().unwrap();
        status.ready = ready;
        status.error = error;
    }

    fn run(&self) {
        loop {
            let generation = self.generation.load(Ordering::Relaxed);
            let cfg = self.cfg.lock().unwrap().clone();
            let unchanged = || self.generation.load(Ordering::Relaxed) == generation;
            if !cfg.enabled {
                self.set_state(false, None);
                while unchanged() {
                    thread::sleep(POLL);
                }
                continue;
            }
            match self.watch(&cfg, &unchanged) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("[button] {e}");
                    self.set_state(false, Some(e));
                    let mut waited = Duration::ZERO;
                    while unchanged() && waited < RETRY {
                        thread::sleep(POLL);
                        waited += POLL;
                    }
                }
            }
        }
    }

    /// Returns once the settings change
    fn watch(&self, cfg: &ButtonSettings, unchanged: &dyn Fn() -> bool) -> Result<(), String> {
        let button = Request::builder()
            .on_chip(CHIP)
            .with_consumer("ystreamer-record")
            .with_line(cfg.pin as u32)
            .as_input()
            .with_bias(Bias::PullUp)
            .with_edge_detection(EdgeDetection::BothEdges)
            .with_debounce_period(DEBOUNCE)
            .request()
            .map_err(|e| format!("GPIO{} can't be used for the button: {e}", cfg.pin))?;
        let led = match cfg.led_pin {
            Some(pin) => Some((
                Request::builder()
                    .on_chip(CHIP)
                    .with_consumer("ystreamer-record-led")
                    .with_line(pin as u32)
                    .as_output(Value::Inactive)
                    .request()
                    .map_err(|e| format!("GPIO{pin} can't be used for the light: {e}"))?,
                pin as u32,
            )),
            None => None,
        };
        println!("[button] record button on GPIO{}", cfg.pin);
        self.set_state(true, None);

        // A switch that's already closed means record, as after a power cut.
        // The pull-up has only just been turned on, so let the pin settle
        if cfg.kind == ButtonKind::Switch {
            thread::sleep(DEBOUNCE * 2);
            let level = button.value(cfg.pin as u32).map_err(|e| e.to_string())?;
            self.record(level == Value::Inactive);
        }

        let mut lit = false;
        while unchanged() {
            if button.wait_edge_event(POLL).map_err(|e| e.to_string())? {
                let closed =
                    button.read_edge_event().map_err(|e| e.to_string())?.kind == EdgeKind::Falling;
                match cfg.kind {
                    ButtonKind::Push if closed => {
                        self.status.lock().unwrap().presses += 1;
                        self.record(!self.recorder.is_active());
                    }
                    ButtonKind::Push => {}
                    ButtonKind::Switch => {
                        self.status.lock().unwrap().presses += 1;
                        self.record(closed);
                    }
                }
            }
            let active = self.recorder.is_active();
            if let Some((led, pin)) = &led
                && active != lit
            {
                let value = if active {
                    Value::Active
                } else {
                    Value::Inactive
                };
                led.set_value(*pin, value).map_err(|e| e.to_string())?;
                lit = active;
            }
        }
        Ok(())
    }

    fn record(&self, on: bool) {
        if on == self.recorder.is_active() {
            return;
        }
        let result = if on {
            self.recorder.start().map(|_| "started")
        } else {
            self.recorder.stop().map(|_| "stopped")
        };
        match result {
            Ok(what) => println!("[button] recording {what}"),
            Err(e) => eprintln!("[button] {e}"),
        }
    }
}

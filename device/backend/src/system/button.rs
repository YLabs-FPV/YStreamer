use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpiocdev::Request;
use gpiocdev::line::{Bias, EdgeDetection, EdgeKind, Value};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::output::recording::Recorder;
use crate::settings::{ButtonKind, ButtonSettings, InputButtonSettings};

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

/// The part of a button's settings that says how it's wired
#[derive(Clone, PartialEq)]
pub struct Wiring {
    pub enabled: bool,
    pub kind: ButtonKind,
    pub pin: u8,
    pub led_pin: Option<u8>,
}

impl From<&ButtonSettings> for Wiring {
    fn from(cfg: &ButtonSettings) -> Self {
        Self {
            enabled: cfg.enabled,
            kind: cfg.kind,
            pin: cfg.pin,
            led_pin: cfg.led_pin,
        }
    }
}

impl From<&InputButtonSettings> for Wiring {
    fn from(cfg: &InputButtonSettings) -> Self {
        Self {
            enabled: cfg.enabled,
            kind: cfg.kind,
            pin: cfg.pin,
            led_pin: None,
        }
    }
}

/// What a button does when it's used
pub trait Action: Send + Sync + 'static {
    /// For the logs and the GPIO consumer name
    const NAME: &'static str;
    /// A push button pressed
    fn pressed(&self);
    /// A switch flipped, or found in this position when the pin opens
    fn switched(&self, closed: bool);
    /// Whether the light should be on
    fn lit(&self) -> bool {
        false
    }
}

pub struct Button<A: Action> {
    cfg: Mutex<Wiring>,
    /// Bumped on every settings change, so the thread reopens its pins
    generation: AtomicU64,
    status: Mutex<ButtonStatus>,
    action: A,
}

pub type RecordButton = Button<Record>;
pub type InputButton = Button<SwitchInput>;

impl<A: Action> Button<A> {
    pub fn start(cfg: Wiring, action: A) -> Arc<Self> {
        let this = Arc::new(Self {
            cfg: Mutex::new(cfg),
            generation: AtomicU64::new(0),
            status: Mutex::new(ButtonStatus::default()),
            action,
        });
        let worker = Arc::clone(&this);
        thread::spawn(move || worker.run());
        this
    }

    pub fn configure(&self, cfg: Wiring) {
        let mut current = self.cfg.lock().unwrap();
        if *current != cfg {
            *current = cfg;
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
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
    fn watch(&self, cfg: &Wiring, unchanged: &dyn Fn() -> bool) -> Result<(), String> {
        let button = Request::builder()
            .on_chip(CHIP)
            .with_consumer(format!("ystreamer-{}", A::NAME))
            .with_line(cfg.pin as u32)
            .as_input()
            .with_bias(Bias::PullUp)
            .with_edge_detection(EdgeDetection::BothEdges)
            .with_debounce_period(DEBOUNCE)
            .request()
            .map_err(|e| {
                format!(
                    "GPIO{} can't be used for the {} button: {e}",
                    cfg.pin,
                    A::NAME
                )
            })?;
        let led = match cfg.led_pin {
            Some(pin) => Some((
                Request::builder()
                    .on_chip(CHIP)
                    .with_consumer(format!("ystreamer-{}-led", A::NAME))
                    .with_line(pin as u32)
                    .as_output(Value::Inactive)
                    .request()
                    .map_err(|e| format!("GPIO{pin} can't be used for the light: {e}"))?,
                pin as u32,
            )),
            None => None,
        };
        println!("[button] {} button on GPIO{}", A::NAME, cfg.pin);
        self.set_state(true, None);

        // A switch's position counts from the start, as after a power cut.
        // The pull-up has only just been turned on, so let the pin settle
        if cfg.kind == ButtonKind::Switch {
            thread::sleep(DEBOUNCE * 2);
            let level = button.value(cfg.pin as u32).map_err(|e| e.to_string())?;
            self.action.switched(level == Value::Inactive);
        }

        let mut lit = false;
        while unchanged() {
            if button.wait_edge_event(POLL).map_err(|e| e.to_string())? {
                let closed =
                    button.read_edge_event().map_err(|e| e.to_string())?.kind == EdgeKind::Falling;
                match cfg.kind {
                    ButtonKind::Push if closed => {
                        self.status.lock().unwrap().presses += 1;
                        self.action.pressed();
                    }
                    ButtonKind::Push => {}
                    ButtonKind::Switch => {
                        self.status.lock().unwrap().presses += 1;
                        self.action.switched(closed);
                    }
                }
            }
            let on = self.action.lit();
            if let Some((led, pin)) = &led
                && on != lit
            {
                let value = if on { Value::Active } else { Value::Inactive };
                led.set_value(*pin, value).map_err(|e| e.to_string())?;
                lit = on;
            }
        }
        Ok(())
    }
}

/// Starts and stops recording
pub struct Record(pub Arc<Recorder>);

impl Record {
    fn record(&self, on: bool) {
        if on == self.0.is_active() {
            return;
        }
        let result = if on {
            self.0.start().map(|_| "started")
        } else {
            self.0.stop().map(|_| "stopped")
        };
        match result {
            Ok(what) => println!("[button] recording {what}"),
            Err(e) => eprintln!("[button] {e}"),
        }
    }
}

impl Action for Record {
    const NAME: &'static str = "record";

    fn pressed(&self) {
        self.record(!self.0.is_active());
    }

    fn switched(&self, closed: bool) {
        self.record(closed);
    }

    fn lit(&self) -> bool {
        self.0.is_active()
    }
}

/// What the input button asks for. The web server saves it, like a change
/// made on the Input page
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputRequest {
    Toggle,
    /// A switch's position
    Switch {
        closed: bool,
    },
}

/// Swaps between the two inputs in the settings
pub struct SwitchInput(pub mpsc::UnboundedSender<InputRequest>);

impl Action for SwitchInput {
    const NAME: &'static str = "input";

    fn pressed(&self) {
        let _ = self.0.send(InputRequest::Toggle);
    }

    fn switched(&self, closed: bool) {
        let _ = self.0.send(InputRequest::Switch { closed });
    }
}

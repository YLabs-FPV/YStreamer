use std::io::{BufRead, IsTerminal, Read, Write};
use std::net::TcpStream;
use std::process::Command as Process;
use std::time::Duration;

use crate::settings;
use crate::system::{reset, unit};
use crate::web::auth::{Auth, MIN_PASSWORD_LEN};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: ystreamer <command>

  status             Whether YStreamer is running and where to open it
  restart            Restart YStreamer
  logs [-f]          Show what YStreamer has logged; -f keeps following
  password           Set the web interface's password
  password --clear   Remove the web interface's password
  reset [--yes]      Put all settings and the password back to the defaults
  version            Show the version
  help               Show this

restart, password and reset need sudo.
`ystreamer run` is YStreamer itself, which the system service starts.";

#[derive(Debug, PartialEq)]
pub enum Command {
    /// The program proper, as the service starts it
    Run,
    Help,
    Version,
    Status,
    Restart,
    Logs {
        follow: bool,
    },
    Password {
        clear: bool,
    },
    Reset {
        confirmed: bool,
    },
}

impl Command {
    /// Nothing at all means help, not the program: someone typing the bare
    /// name would otherwise start a second copy fighting the first
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(match args.as_slice() {
            [] | ["help" | "--help" | "-h"] => Self::Help,
            ["run"] => Self::Run,
            ["version" | "--version" | "-V"] => Self::Version,
            ["status"] => Self::Status,
            ["restart"] => Self::Restart,
            ["logs"] => Self::Logs { follow: false },
            ["logs", "-f" | "--follow"] => Self::Logs { follow: true },
            ["password"] => Self::Password { clear: false },
            ["password", "--clear"] => Self::Password { clear: true },
            ["reset"] => Self::Reset { confirmed: false },
            ["reset", "-y" | "--yes"] => Self::Reset { confirmed: true },
            [command, ..] if !KNOWN.contains(command) => {
                return Err(format!("Unknown command '{command}'"));
            }
            [command, rest @ ..] => {
                return Err(format!("'{command}' doesn't take '{}'", rest.join(" ")));
            }
        })
    }
}

const KNOWN: [&str; 8] = [
    "run", "help", "version", "status", "restart", "logs", "password", "reset",
];

/// The exit code. `Run` isn't handled here
pub fn execute(command: Command) -> i32 {
    let result = match command {
        Command::Run | Command::Help => {
            println!("{HELP}");
            Ok(())
        }
        Command::Version => {
            println!("{VERSION}");
            Ok(())
        }
        Command::Status => {
            status();
            Ok(())
        }
        Command::Restart => as_root().and_then(|()| systemctl("restart")),
        Command::Logs { follow } => logs(follow),
        Command::Password { clear } => as_root().and_then(|()| password(clear)),
        Command::Reset { confirmed } => as_root().and_then(|()| reset_settings(confirmed)),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub fn usage_error(message: &str) -> i32 {
    eprintln!("{message}\n\n{HELP}");
    2
}

fn as_root() -> Result<(), String> {
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err("This needs root: run it with sudo".into())
    }
}

fn service_active() -> bool {
    Process::new("systemctl")
        .args(["is-active", "--quiet", &unit()])
        .status()
        .is_ok_and(|s| s.success())
}

fn systemctl(action: &str) -> Result<(), String> {
    let done = Process::new("systemctl")
        .args([action, &unit()])
        .status()
        .map_err(|e| format!("Couldn't run systemctl: {e}"))?;
    if done.success() {
        Ok(())
    } else {
        Err(format!("systemctl {action} {} failed", unit()))
    }
}

fn status() {
    println!("YStreamer {VERSION}");
    let running = running_version();
    match (&running, service_active()) {
        (Some(version), _) if version != VERSION => {
            println!("Service:        running version {version}")
        }
        (Some(_), _) => println!("Service:        running"),
        (None, true) => println!("Service:        started, but the web interface isn't answering"),
        (None, false) => println!("Service:        not running"),
    }

    let mut addresses = vec![format!("http://{}.local", settings::current_hostname())];
    if let Ok(out) = Process::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
    {
        addresses.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(reachable_address)
                .map(|ip| format!("http://{ip}")),
        );
    }
    println!("Web interface:  {}", addresses.join("  "));

    if unsafe { libc::geteuid() } != 0 {
        println!("Password:       run with sudo to see");
    } else if Auth::load(auth_path()).enabled() {
        println!("Password:       set");
    } else {
        println!("Password:       none, anyone on the network can change settings");
    }
}

/// The address in a line of `ip -o addr`, unless it belongs to a container
/// bridge, which nobody outside this device can reach
fn reachable_address(line: &str) -> Option<&str> {
    let mut fields = line.split_whitespace();
    let interface = fields.nth(1)?;
    if ["docker", "br-", "veth"]
        .iter()
        .any(|p| interface.starts_with(p))
    {
        return None;
    }
    let address = fields.skip_while(|f| *f != "inet").nth(1)?;
    address.split('/').next()
}

/// What the service answers on its health address, on either port it uses
fn running_version() -> Option<String> {
    [80, 8080].into_iter().find_map(|port| {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .ok()?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).ok()?;
        version_in(&reply)
    })
}

fn version_in(reply: &str) -> Option<String> {
    let rest = reply.split_once(r#""version":""#)?.1;
    Some(rest.split_once('"')?.0.to_string())
}

fn logs(follow: bool) -> Result<(), String> {
    let unit = unit();
    let mut args = vec!["-u", unit.as_str(), "-o", "short-iso"];
    if follow {
        args.extend(["-n", "50", "-f"]);
    } else {
        args.extend(["-n", "200", "--no-pager"]);
    }
    Process::new("journalctl")
        .args(args)
        .status()
        .map(|_| ())
        .map_err(|e| format!("Couldn't run journalctl: {e}"))
}

fn auth_path() -> std::path::PathBuf {
    settings::config_path().with_file_name("auth.json")
}

fn password(clear: bool) -> Result<(), String> {
    let new = if clear { None } else { Some(ask_password()?) };
    let auth = Auth::load(auth_path());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(auth.set_password(new))?;

    // The running service only reads the login when told to
    if service_active() && systemctl("reload").is_err() {
        println!("Saved. Restart YStreamer for it to take effect: sudo ystreamer restart");
    } else if clear {
        println!("The web interface no longer asks for a password.");
    } else {
        println!("Password set. Browsers that were logged in need to log in again.");
    }
    Ok(())
}

fn ask_password() -> Result<String, String> {
    let stdin = std::io::stdin();
    // Piped in, for scripts: one line, no questions
    if !stdin.is_terminal() {
        let mut line = String::new();
        stdin
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        return check_length(line.trim_end_matches(['\r', '\n']).to_string());
    }
    let first = check_length(prompt_hidden("New password: ")?)?;
    if prompt_hidden("Again: ")? != first {
        return Err("Those don't match; nothing was changed".into());
    }
    Ok(first)
}

fn check_length(password: String) -> Result<String, String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("Use at least {MIN_PASSWORD_LEN} characters"));
    }
    Ok(password)
}

/// Read a line from the terminal without showing it
fn prompt_hidden(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    std::io::stdout().flush().map_err(|e| e.to_string())?;

    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    let hidden = unsafe { libc::tcgetattr(0, &mut term) } == 0;
    if hidden {
        let mut quiet = term;
        quiet.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &quiet) };
    }
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if hidden {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) };
        println!();
    }
    read.map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn reset_settings(confirmed: bool) -> Result<(), String> {
    if !confirmed {
        println!("This puts every setting and the web interface's password back to the defaults.");
        println!("Recordings, the logo and the splash image stay.");
        print!("Continue? [y/N] ");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let mut answer = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| e.to_string())?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Err("Nothing was changed.".into());
        }
    }
    // Done by the service as it starts, which also puts the network back
    reset::request(&settings::config_path())?;
    if service_active() {
        systemctl("restart")?;
        println!("Settings reset. YStreamer is restarting with the defaults.");
    } else {
        println!("YStreamer isn't running; the settings are reset when it next starts.");
    }
    Ok(())
}

#[cfg(test)]
#[path = "cli.test.rs"]
mod tests;

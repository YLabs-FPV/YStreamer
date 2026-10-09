use super::*;

fn parse(line: &str) -> Result<Command, String> {
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    Command::parse(&args)
}

#[test]
fn bare_name_shows_help_instead_of_starting_a_second_copy() {
    assert_eq!(parse(""), Ok(Command::Help));
    assert_eq!(parse("--help"), Ok(Command::Help));
    assert_eq!(parse("run"), Ok(Command::Run));
}

#[test]
fn reads_each_command() {
    assert_eq!(parse("status"), Ok(Command::Status));
    assert_eq!(parse("restart"), Ok(Command::Restart));
    assert_eq!(parse("version"), Ok(Command::Version));
    assert_eq!(parse("logs"), Ok(Command::Logs { follow: false }));
    assert_eq!(parse("logs -f"), Ok(Command::Logs { follow: true }));
    assert_eq!(parse("password"), Ok(Command::Password { clear: false }));
    assert_eq!(
        parse("password --clear"),
        Ok(Command::Password { clear: true })
    );
    assert_eq!(parse("reset"), Ok(Command::Reset { confirmed: false }));
    assert_eq!(parse("reset --yes"), Ok(Command::Reset { confirmed: true }));
}

#[test]
fn says_what_it_did_not_understand() {
    assert_eq!(parse("reboot"), Err("Unknown command 'reboot'".into()));
    assert_eq!(parse("reset now"), Err("'reset' doesn't take 'now'".into()));
    assert_eq!(parse("run fast"), Err("'run' doesn't take 'fast'".into()));
}

#[test]
fn finds_the_version_in_a_health_reply() {
    let reply = "HTTP/1.0 200 OK\r\ncontent-type: application/json\r\n\r\n{\"status\":\"ok\",\"version\":\"0.2.0\"}";
    assert_eq!(version_in(reply), Some("0.2.0".into()));
    assert_eq!(version_in("HTTP/1.0 200 OK\r\n\r\nok"), None);
}

#[test]
fn short_passwords_are_refused() {
    assert!(check_length("12345".into()).is_err());
    assert_eq!(check_length("123456".into()), Ok("123456".into()));
}

#[test]
fn lists_addresses_people_can_reach() {
    let wifi = "3: wlan0    inet 10.50.10.103/24 brd 10.50.10.255 scope global dynamic wlan0";
    let docker = "4: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0";
    let bridge = "5: br-1a2b3c    inet 172.18.0.1/16 scope global br-1a2b3c";
    assert_eq!(reachable_address(wifi), Some("10.50.10.103"));
    assert_eq!(reachable_address(docker), None);
    assert_eq!(reachable_address(bridge), None);
    assert_eq!(reachable_address(""), None);
}

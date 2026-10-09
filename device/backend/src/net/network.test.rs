use super::*;

#[test]
fn country_in_force_is_the_first_block() {
    let reg = "global\ncountry ES: DFS-ETSI\n\t(2400 - 2483 @ 40), (N/A, 20), (N/A)\n\nphy#0\ncountry 99: DFS-UNSET\n";
    assert_eq!(current_country(reg), Some("ES".into()));
}

#[test]
fn worldwide_fallback_is_no_country() {
    assert_eq!(current_country("global\ncountry 00: DFS-UNSET\n"), None);
    assert_eq!(current_country(""), None);
}

#[test]
fn countries_come_sorted_by_name_without_the_comments() {
    let table = "# a comment\n#code\tname\nES\tSpain\nDE\tGermany\nBO\tBolivia\n";
    let names: Vec<_> = parse_countries(table)
        .into_iter()
        .map(|c| (c.code, c.name))
        .collect();
    assert_eq!(
        names,
        [
            ("BO".into(), "Bolivia".into()),
            ("DE".into(), "Germany".into()),
            ("ES".into(), "Spain".into()),
        ]
    );
}

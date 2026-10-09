#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Goggles,
    Aircraft,
    Remote,
    Other,
    Unknown,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub code: String,
    pub name: String,
    pub kind: Kind,
    pub address: u8,
}

// DJI's product codes, from DJI Fly
const KNOWN: &[(&str, &str, Kind)] = &[
    ("GL150", "FPV Goggles V1", Kind::Goggles),
    ("GL170", "Goggles FPV Racer", Kind::Goggles),
    ("GL811", "Goggles Racing Edition", Kind::Goggles),
    ("GP150", "FPV Goggles V2", Kind::Goggles),
    ("P1GS", "Goggles Standard", Kind::Goggles),
    ("ZV300", "Goggles N3", Kind::Goggles),
    ("ZV900", "Goggles 2", Kind::Goggles),
    ("ZV901", "Goggles Integra", Kind::Goggles),
    ("ZV902", "Goggles 3", Kind::Goggles),
    ("AC101", "Osmo Action", Kind::Other),
    ("AC103", "Osmo Action 2", Kind::Other),
    ("AC202", "Osmo Action 3", Kind::Other),
    ("AC203", "Osmo Action 4", Kind::Other),
    ("DLG30A", "N3 Flight Controller", Kind::Other),
    ("EA220E", "Matrice 3D", Kind::Aircraft),
    ("EA220T", "Matrice 3D Thermal", Kind::Aircraft),
    ("EP600", "Zenmuse P1", Kind::Other),
    ("EP800", "Zenmuse L1", Kind::Other),
    ("EP810", "Zenmuse L2", Kind::Other),
    ("EVO1", "EVO", Kind::Aircraft),
    ("EVO2", "EVO 2", Kind::Aircraft),
    ("GD610", "Zenmuse H20", Kind::Other),
    ("GD612", "Zenmuse H20N", Kind::Other),
    ("GD620", "X-Port", Kind::Other),
    ("HG200", "Osmo", Kind::Other),
    ("HG210", "Osmo Pocket", Kind::Other),
    ("HG211", "Osmo Pocket 2", Kind::Other),
    ("HG212", "Osmo Pocket 3", Kind::Other),
    ("HG330VTX", "Ronin 4D Video Transmitter", Kind::Other),
    ("HG704", "RS 4 Mini", Kind::Other),
    ("HG714", "RS 4 Handheld Gimbal", Kind::Other),
    ("LT150", "FPV Air Unit Lite", Kind::Aircraft),
    ("M600", "Matrice 600 Pro", Kind::Aircraft),
    ("M601", "Matrice 600", Kind::Aircraft),
    ("PM320", "Matrice 30/30T", Kind::Aircraft),
    ("PM410", "Matrice 200", Kind::Aircraft),
    ("PM420", "Matrice 200 V2", Kind::Aircraft),
    ("PM430", "Matrice 300", Kind::Aircraft),
    ("PM431", "Matrice 350 RTK", Kind::Aircraft),
    ("R400", "D-RTK GNSS", Kind::Other),
    ("RC151", "RC-N2", Kind::Remote),
    ("RC151B-WA150", "RC-N3", Kind::Remote),
    ("RC221", "RC Motion 3", Kind::Remote),
    ("RC331", "DJI RC 2", Kind::Remote),
    ("RC430", "Matrice 300 RC", Kind::Remote),
    ("RC520", "RC Pro 2", Kind::Remote),
    ("RC701", "Enterprise RC", Kind::Remote),
    ("RCP501", "Unknown remote (RCP501)", Kind::Remote),
    ("RM010", "Unknown remote (RM010)", Kind::Remote),
    ("RM220", "RC Motion 2", Kind::Remote),
    ("RM330", "DJI RC", Kind::Remote),
    ("RM500", "Smart Controller", Kind::Remote),
    ("RM510", "RC Pro", Kind::Remote),
    ("RM510B", "RC Pro Enterprise", Kind::Remote),
    ("RM510BV", "RC Pro Enterprise", Kind::Remote),
    ("RM700", "RC Plus", Kind::Remote),
    ("RM700_ENTERPRISE", "RC Plus Enterprise", Kind::Remote),
    ("WA020", "Neo 2", Kind::Aircraft),
    ("WA140", "Mini 4 Pro", Kind::Aircraft),
    ("WA141", "Flip", Kind::Aircraft),
    ("WA150", "Mini 5 Pro", Kind::Aircraft),
    ("WA1617", "Mini 4K", Kind::Aircraft),
    ("WA233", "Air 3", Kind::Aircraft),
    ("WA234", "Air 3S", Kind::Aircraft),
    ("WA341", "Mavic 4 Pro", Kind::Aircraft),
    ("WA345E", "Matrice 4 Enterprise", Kind::Aircraft),
    ("WA345T", "Matrice 4 Thermal", Kind::Aircraft),
    ("WA520", "Avata 2", Kind::Aircraft),
    ("WA521", "Neo", Kind::Aircraft),
    ("WM100", "Spark", Kind::Aircraft),
    ("WM150", "FPV Air Unit", Kind::Aircraft),
    ("WM160", "Mavic Mini", Kind::Aircraft),
    ("WM1605", "Mini SE", Kind::Aircraft),
    ("WM161", "Mini 2", Kind::Aircraft),
    ("WM1615", "Mini 2 SE", Kind::Aircraft),
    ("WM162", "Mini 3 Pro", Kind::Aircraft),
    ("WM163", "Mini 3", Kind::Aircraft),
    ("WM169", "Avata", Kind::Aircraft),
    ("WM1695", "O3 Air Unit", Kind::Aircraft),
    ("WM170", "DJI FPV", Kind::Aircraft),
    ("WM220", "Mavic Pro", Kind::Aircraft),
    ("WM222", "Unknown aircraft (WM222)", Kind::Aircraft),
    ("WM230", "Mavic Air", Kind::Aircraft),
    ("WM231", "Mavic Air 2", Kind::Aircraft),
    ("WM232", "Air 2S", Kind::Aircraft),
    ("WM240", "Mavic 2 Pro/Zoom", Kind::Aircraft),
    ("WM245", "Mavic 2 Enterprise", Kind::Aircraft),
    ("WM246", "Mavic 2 Enterprise Dual", Kind::Aircraft),
    ("WM247", "Mavic 2 Enterprise Advanced", Kind::Aircraft),
    ("WM260", "Mavic 3", Kind::Aircraft),
    ("WM2605", "Mavic 3 Classic", Kind::Aircraft),
    ("WM261", "Mavic 3 Pro", Kind::Aircraft),
    ("WM265E", "Mavic 3 Enterprise", Kind::Aircraft),
    ("WM265M", "Mavic 3 Multispectral", Kind::Aircraft),
    ("WM265T", "Mavic 3 Thermal", Kind::Aircraft),
    ("WM321", "Phantom 3 Standard", Kind::Aircraft),
    ("WM322", "Phantom 3 Advanced", Kind::Aircraft),
    ("WM323", "Phantom 3 Professional", Kind::Aircraft),
    ("WM325", "Phantom 3 4K", Kind::Aircraft),
    ("WM330", "Phantom 4", Kind::Aircraft),
    ("WM331", "Phantom 4 Pro", Kind::Aircraft),
    ("WM332", "Phantom 4 Advanced", Kind::Aircraft),
    ("WM334", "Phantom 4 RTK", Kind::Aircraft),
    ("WM335", "Phantom 4 Pro V2.0", Kind::Aircraft),
    ("WM336", "Phantom 4 Multispectral", Kind::Aircraft),
    ("WM600", "Inspire 1", Kind::Aircraft),
    ("WM610", "Inspire 1 Pro", Kind::Aircraft),
    ("WM620", "Inspire 2", Kind::Aircraft),
    ("WM630", "Inspire 3", Kind::Aircraft),
    ("ZA530", "O4 Air Unit", Kind::Aircraft),
    ("ZA5305", "O4 Air Unit Pro", Kind::Aircraft),
    ("ZV811", "OcuSync Air System", Kind::Aircraft),
];

pub fn from_identity(address: u8, record: &[u8]) -> Option<Device> {
    let raw = record.get(..32)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let code = std::str::from_utf8(&raw[..end]).ok()?.trim();
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let (name, kind) = KNOWN
        .iter()
        .find(|(c, _, _)| c.eq_ignore_ascii_case(code))
        .map(|&(_, name, kind)| (name.to_string(), kind))
        .unwrap_or_else(|| (code.to_string(), Kind::Unknown));
    Some(Device {
        code: code.to_string(),
        name,
        kind,
        address,
    })
}

#[cfg(test)]
#[path = "devices.test.rs"]
mod tests;

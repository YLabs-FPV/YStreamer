use super::*;

const VIDEO: Rect = Rect {
    x: 0,
    y: 0,
    w: 1920,
    h: 1080,
};

fn cfg(x_percent: f32, y_percent: f32, size_percent: f32) -> OverlaySettings {
    OverlaySettings {
        x_percent,
        y_percent,
        size_percent,
        ..Default::default()
    }
}

#[test]
fn keeps_the_logo_shape_and_reaches_the_edges() {
    // 2:1 logo, 10% of 1920 wide
    let r = place(VIDEO, (400, 200), &cfg(100.0, 0.0, 10.0));
    assert_eq!(
        r,
        Rect {
            x: 1920 - 192,
            y: 0,
            w: 192,
            h: 96
        }
    );
    let r = place(VIDEO, (400, 200), &cfg(0.0, 100.0, 10.0));
    assert_eq!(
        r,
        Rect {
            x: 0,
            y: 1080 - 96,
            w: 192,
            h: 96
        }
    );
    let r = place(VIDEO, (400, 200), &cfg(50.0, 50.0, 10.0));
    assert_eq!(
        r,
        Rect {
            x: (1920 - 192) / 2,
            y: (1080 - 96) / 2,
            w: 192,
            h: 96
        }
    );
}

#[test]
fn follows_a_letterboxed_picture() {
    // 4:3 picture centred on a 16:9 screen
    let video = Rect {
        x: 240,
        y: 0,
        w: 1440,
        h: 1080,
    };
    let r = place(video, (100, 100), &cfg(0.0, 0.0, 10.0));
    assert_eq!(
        r,
        Rect {
            x: 240,
            y: 0,
            w: 144,
            h: 144
        }
    );
}

#[test]
fn shrinks_a_logo_taller_than_the_picture() {
    let r = place(VIDEO, (100, 100), &cfg(50.0, 50.0, 100.0));
    assert_eq!(
        r,
        Rect {
            x: 420,
            y: 0,
            w: 1080,
            h: 1080
        }
    );
}

#[test]
fn rejects_non_png() {
    assert!(decode(b"GIF89a....").is_err());
}

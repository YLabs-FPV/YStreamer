# Contributing to YStreamer

Thanks for your interest - contributions are welcome, whether that's code, docs,
or simply reporting how YStreamer behaves with your goggles or camera.

## Ways to help

- **Report your hardware.** Tried YStreamer with your goggles, air unit or a USB
  camera? Tell us how it went with the
  [hardware compatibility report](https://github.com/YLabs-FPV/YStreamer/issues/new?template=hardware_report.yml).
  It's genuinely useful even when everything works - it's how the list of
  supported inputs grows, and the DJI FPV Goggles V1 / V2 in particular still
  need testing.
- **Report a bug** with the [bug report](https://github.com/YLabs-FPV/YStreamer/issues/new?template=bug_report.yml)
  template. The lines from the **Logs** page (or `ystreamer logs`) help a lot.
- **Suggest a feature** with the [feature request](https://github.com/YLabs-FPV/YStreamer/issues/new?template=feature_request.yml)
  template.
- **Improve the docs or code** with a pull request (see below).

## The golden rule: test on real hardware

YStreamer handles live video from real goggles, on a real Raspberry Pi, and a lot
of what matters only shows up there: timing, the USB link, the hardware decoder
and encoder, power. **Any change to how the device behaves must be tested on a
Raspberry Pi 4 before it's merged.** In your pull request, say how YStreamer was
installed (image or install script), which video source you used, and what you
verified.

**AI-assisted contributions are welcome** - use whatever tools help you. The same
rule applies without exception: if it changes how the device behaves, you must
have run it on a real Pi and confirmed it works. Untested, "looks correct"
changes will be asked to prove themselves on hardware first.

Documentation-only or website-only changes don't need hardware testing - just make
sure the site builds (`mise run site:build`).

## Development setup

Everything runs through [mise](https://mise.jdx.dev), which also installs the
toolchains the project pins (Rust, Node, pnpm). `mise tasks` lists every task.

### Backend (`device/backend/`)

The YStreamer service, in Rust, with GStreamer for the video. To run it with
the goggles, the USB-C link and HDMI, it needs a Raspberry Pi:

```bash
mise run backend:deps         # GStreamer and GLib headers (Debian / Raspberry Pi OS)
mise run backend:boot-config  # boot settings for the USB-C link and the encoder; reboot after
mise run backend:run          # build and run it as root, as the hardware and port 80 need
mise run backend:test
```

`backend:boot-config` adds to `/boot/firmware/config.txt` what installing the
package would: `dtoverlay=dwc2,dr_mode=peripheral`, which puts the USB-C port in
device mode for the goggles, and `gpu_mem=256` for the hardware encoder. It keeps
the original as `config.txt.before-ystreamer` and changes nothing if the settings
are already there. Without it the backend still runs, but logs that there's no
link to the goggles.

Stop the installed service first (`sudo systemctl stop ystreamer`), or the two
will fight over the goggles and port 80.

### Web interface (`device/frontend/`)

Svelte, compiled into the backend for releases.

```bash
mise run frontend:install
mise run frontend:dev      # dev server with hot reload
mise run frontend:check    # type checks
```

### Packages and the image

```bash
mise run device:package    # build the .deb and sign it
mise run image:build       # SD card image from the newest .deb (needs Docker)
mise run image:full        # both, from source
mise run image:test        # the data-partition setup, against simulated SD cards
```

Signing needs the release key, which only the maintainer has. To try an update
on a device without it, see `device:serve-release` in `mise.toml`.

### Site (`site/`)

The website and the documentation.

```bash
mise run site:install
mise run site:dev          # local dev server
mise run site:build        # production build
```

The docs are in `site/src/content/docs/en/`, one folder per section.

## Pull request checklist

- [ ] Changes to how the device behaves are **tested on a Raspberry Pi 4** (state
      the install method, video source, and what you verified).
- [ ] `mise run lint` and `mise run backend:test` pass.
- [ ] Code follows the style of the surrounding files (`rustfmt` and Clippy for the
      backend, Prettier for the web interface).
- [ ] Docs updated if behaviour changed.
- [ ] Commits are focused and the PR description explains the "why".

## License

By contributing, you agree that your contributions are licensed under the project's
[GPL-3.0](LICENSE) license.

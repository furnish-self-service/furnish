# UI test environment: the real furnish GUI under Xvfb, driven via tauri-driver (WebDriver).
# Ubuntu, because it packages WebKitWebDriver (webkit2gtk-driver); Arch does not.

FROM rust:1-bookworm AS driver
RUN cargo install tauri-driver --locked --version 2.1.0

# Runtime: what the tests need to run a prebuilt furnish-gui (CI builds it on ubuntu-24.04).
FROM ubuntu:24.04 AS runtime
COPY --chmod=0755 install-deps.sh /usr/local/bin/install-deps
RUN install-deps --no-install-recommends libwebkit2gtk-4.1-0 libayatana-appindicator3-1 webkit2gtk-driver xvfb git ca-certificates fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
COPY --from=driver /usr/local/cargo/bin/tauri-driver /usr/local/bin/

# Build: adds the toolchain to compile furnish-gui for this image locally (glibc must match).
FROM runtime AS build
RUN install-deps --no-install-recommends build-essential pkg-config curl libwebkit2gtk-4.1-dev librsvg2-dev \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal && chmod -R a+rwX /opt/cargo /opt/rustup

# Screenshots for the website and docs (docs/screenshots/run.sh): a nicer system font than DejaVu.
FROM build AS screenshots
RUN install-deps --no-install-recommends fonts-inter fonts-noto-color-emoji imagemagick \
    && rm -rf /var/lib/apt/lists/* \
    && printf '<?xml version="1.0"?><fontconfig><alias><family>sans-serif</family><prefer><family>Inter</family></prefer></alias><alias><family>system-ui</family><prefer><family>Inter</family></prefer></alias></fontconfig>' > /etc/fonts/local.conf

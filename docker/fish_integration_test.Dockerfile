FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

# Empty: the distro fish (3.7.x). Set (e.g. 4.9.3): pinned static fish 4 build.
ARG FISH_VERSION=

RUN apt-get update && apt-get install -y \
    curl \
    git \
    python3 \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

COPY docker/install_fish.sh /tmp/install_fish.sh
RUN sh /tmp/install_fish.sh "${FISH_VERSION}"

COPY --from=built-artifact /flyline-standalone /usr/local/bin/flyline-standalone
COPY scripts/flyline.fish /opt/flyline/flyline.fish
COPY docker/fish_integration_test.py /opt/flyline/test.py

RUN chmod +x /usr/local/bin/flyline-standalone \
    && python3 /opt/flyline/test.py

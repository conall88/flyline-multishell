FROM ubuntu:24.04

RUN apt-get update && apt-get install -y \
    zsh \
    git \
    python3 \
    && rm -rf /var/lib/apt/lists/*

RUN zsh --version

COPY --from=built-artifact /flyline-standalone /usr/local/bin/flyline-standalone
COPY --from=built-artifact /libflyline.so /usr/local/lib/libflyline.so
COPY scripts/flyline.zsh /opt/flyline/flyline.zsh
COPY docker/zsh_integration_test.sh /opt/flyline/test.sh

RUN chmod +x /usr/local/bin/flyline-standalone /opt/flyline/test.sh \
    && /opt/flyline/test.sh

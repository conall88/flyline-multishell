FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DOCKER_BASH_VERSION

RUN apt update && apt install -y \
  build-essential \
  ca-certificates \
  curl \
  libreadline-dev \
  libncurses-dev \
  bison \
  yacc \
  && rm -rf /var/lib/apt/lists/*


WORKDIR /tmp/bash-build
RUN case "${DOCKER_BASH_VERSION}" in \
      3.2.*) make_jobs=1 ;; \
      *) make_jobs="$(nproc)" ;; \
    esac \
 && curl --fail --location --show-error --silent --retry 5 \
      -O https://ftp.gnu.org/gnu/bash/bash-${DOCKER_BASH_VERSION}.tar.gz \
 && tar xzf bash-${DOCKER_BASH_VERSION}.tar.gz \
 && cd bash-${DOCKER_BASH_VERSION} \
 && ./configure --prefix=/opt/bash-${DOCKER_BASH_VERSION} --with-readline \
 && make -j"${make_jobs}" \
 && make install \
 && cd .. \
 && rm -rf bash-${DOCKER_BASH_VERSION}* 

RUN rm /bin/bash && ln -s /opt/bash-${DOCKER_BASH_VERSION}/bin/bash /bin/bash

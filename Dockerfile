# Copyright 2025 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http:#www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

ARG BASE_IMAGE=debian:trixie

FROM ${BASE_IMAGE}

RUN apt update && \ 
    apt install -y curl libxkbcommon-dev libwayland-dev devscripts pkg-config

RUN curl --proto '=https' --tlsv1.2 -OsSf \
    https://static.rust-lang.org/rustup/archive/1.27.1/x86_64-unknown-linux-gnu/rustup-init && \
    echo "6aeece6993e902708983b209d04c0d1dbb14ebb405ddb87def578d41f920f56d rustup-init" \
    | sha256sum --check && \
    chmod +x rustup-init && ./rustup-init -y

ENV PATH="$PATH:/root/.cargo/bin"

COPY / /wprs
WORKDIR /wprs

RUN dpkg-buildpackage --sanitize-env -us -uc -b -d -rfakeroot

WORKDIR /

RUN echo "Deb Files Created:" && ls *.deb

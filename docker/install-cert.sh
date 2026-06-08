#!/bin/bash

docker cp \
    docker-reverse-proxy-1:/data/caddy/pki/authorities/local/root.crt \
    /usr/local/share/ca-certificates/root.crt \
  && update-ca-certificates

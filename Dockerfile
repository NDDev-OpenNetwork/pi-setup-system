# syntax=docker/dockerfile:1
# Built from the release artifacts, not compiled again: an image compiled
# separately could differ from the binary published beside it, and then each
# attestation would be true of different bytes.
FROM gcr.io/distroless/cc-debian12:nonroot

ARG TARGETARCH
COPY staging/$TARGETARCH/pi-setup-system /usr/local/bin/pi-setup-system

# Every command takes an explicit --target, so there is no default to set and
# nothing is inferred from a home directory inside the image either.
ENTRYPOINT ["/usr/local/bin/pi-setup-system"]

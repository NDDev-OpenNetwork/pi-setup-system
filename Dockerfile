# syntax=docker/dockerfile:1
# Built from the release artifacts, not compiled again: an image compiled
# separately could differ from the binary published beside it, and then each
# attestation would be true of different bytes.
# Pinned by digest, not by tag. Everything else in this estate is: action SHAs,
# vendor artifacts, the toolchain. A base image named by a mutable tag was the
# one input that could change under the same name between two builds of the same
# commit -- and unlike a stale vendor pin, which is visible as a version number
# going backwards, a republished tag leaves no trace at all.
#
# The digest is the OCI image index the registry computed for `nonroot`, so
# multi-arch resolution is unchanged: it covers amd64, arm, arm64, ppc64le and
# s390x, and `TARGETARCH` still selects from it. Refresh it by asking the
# registry rather than by editing the string:
#
#   crane digest gcr.io/distroless/cc-debian12:nonroot
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

ARG TARGETARCH
COPY staging/$TARGETARCH/pi-setup-system /usr/local/bin/pi-setup-system

# Every command takes an explicit --target, so there is no default to set and
# nothing is inferred from a home directory inside the image either.
ENTRYPOINT ["/usr/local/bin/pi-setup-system"]

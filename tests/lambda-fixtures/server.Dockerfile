# syntax=docker/dockerfile:1
ARG BASE_IMAGE=public.ecr.aws/lambda/provided@sha256:612c98f126c2a95e77a016d56fed50a83055e11d636132ee6ebdd461f01b9617
FROM ${BASE_IMAGE}
USER 0:0
RUN mkdir -p /data /etc/rustack /run/rustack \
    && chown 65532:65532 /data \
    && chmod 0700 /data \
    && touch /run/rustack/docker.sock
COPY --chmod=0555 rustack /rustack
COPY --chmod=0444 config.yaml /etc/rustack/config.yaml
WORKDIR /data
USER 65532:65532
EXPOSE 4566
HEALTHCHECK NONE
ENTRYPOINT ["/rustack"]
CMD []

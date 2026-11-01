FROM public.ecr.aws/lambda/provided:al2023@sha256:612c98f126c2a95e77a016d56fed50a83055e11d636132ee6ebdd461f01b9617
ARG IMAGE_REVISION
ENV IMAGE_FIXTURE_REVISION=${IMAGE_REVISION}
COPY --chmod=0555 bootstrap /image-bootstrap
COPY --chmod=0555 bootstrap /alternate-bootstrap
WORKDIR /var/task
USER 1000:1000
ENTRYPOINT ["/image-bootstrap"]
CMD ["inherited-command"]

FROM public.ecr.aws/lambda/provided:al2023@sha256:612c98f126c2a95e77a016d56fed50a83055e11d636132ee6ebdd461f01b9617
COPY bootstrap /no-runtime
WORKDIR /var/task
USER 1000:1000
ENTRYPOINT ["/no-runtime"]
CMD []

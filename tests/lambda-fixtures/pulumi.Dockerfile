FROM public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79
COPY --from=pulumi-tools / /opt/pulumi/
COPY pulumi/package.json pulumi/package-lock.json /var/task/pulumi/
RUN cd /var/task/pulumi && npm ci --ignore-scripts --no-fund --no-audit --registry=https://registry.npmjs.org --userconfig=/tmp/npm-user --globalconfig=/tmp/npm-global --cache=/tmp/npm-cache && rm -rf /tmp/npm-cache
COPY pulumi/*.js /var/task/pulumi/
COPY iac.js /var/task/iac.js
RUN node /var/task/pulumi/image_identity.js && node --test /var/task/pulumi/*-controls.js && /opt/pulumi/bin/pulumi version
RUN set -eu; image_bytes="$(du -sbx /)"; image_bytes="${image_bytes%%[!0-9]*}"; test "$image_bytes" -le 4294967296; printf 'unpackedImageBytes=%s\n' "$image_bytes"
ENV GOMAXPROCS=1 UV_THREADPOOL_SIZE=2
USER 1000:1000
CMD ["iac.handler"]

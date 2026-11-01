FROM public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79
COPY callback.js ${LAMBDA_TASK_ROOT}/callback.js
ENV AWS_PROFILE=callback-image-poison \
    AWS_ENDPOINT_URL_LAMBDA=http://127.0.0.1:9 \
    AWS_ENDPOINT_URL_SFN=http://127.0.0.1:9 \
    AWS_ENDPOINT_URL_DYNAMODB=http://127.0.0.1:9 \
    AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=true
USER 1000:1000
CMD ["callback.dispatcher"]

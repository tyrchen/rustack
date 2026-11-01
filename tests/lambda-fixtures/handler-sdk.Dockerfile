FROM public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79
COPY handler-sdk.js ${LAMBDA_TASK_ROOT}/handler-sdk.js
ENV AWS_ACCESS_KEY_ID=sdk-image-poison-key \
    AWS_SECRET_ACCESS_KEY=sdk-image-poison-secret \
    AWS_PROFILE=sdk-image-poison \
    AWS_ENDPOINT_URL_S3=http://127.0.0.1:9 \
    AWS_ENDPOINT_URL_DYNAMODB=http://127.0.0.1:9 \
    AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=true \
    AWS_SESSION_TOKEN=sdk-image-poison \
    AWS_CONTAINER_CREDENTIALS_FULL_URI=http://127.0.0.1:9
USER 1000:1000
CMD ["handler-sdk.handler"]

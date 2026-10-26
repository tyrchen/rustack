FROM public.ecr.aws/lambda/python:3.13@sha256:e0237850eecd760896365e768bac1526ea01098e325fa590e5bc735b26d5dbb9
COPY handler.py /var/task/handler.py
ENV AWS_LAMBDA_RUNTIME_API=outside:9001 \
    AWS_LAMBDA_FUNCTION_NAME=image-default \
    AWS_REGION=image-region \
    TEST_VALUE=image-default
USER 1000:1000
CMD ["handler.handler"]

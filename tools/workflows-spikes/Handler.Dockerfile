FROM public.ecr.aws/lambda/python:3.13@sha256:e0237850eecd760896365e768bac1526ea01098e325fa590e5bc735b26d5dbb9
COPY handler.py /var/task/handler.py
USER 1000:1000
CMD ["handler.handler"]

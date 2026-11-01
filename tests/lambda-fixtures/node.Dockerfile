FROM public.ecr.aws/lambda/nodejs:24@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79
COPY node.js /var/task/node.js
USER 1000:1000
CMD ["node.handler"]

# Rustack Hackathon App Pulumi Stack

This Pulumi program deploys a realistic AWS serverless hackathon application
topology into Rustack:

- CloudFront routes the frontend to S3.
- CloudFront routes API traffic to API Gateway V2, Lambda, DynamoDB, and S3.
- CloudFront routes protected data through a token-checking CloudFront Function
  to a private S3 bucket.
- Secrets are provisioned in SSM Parameter Store.
- Image uploads flow through Lambda, S3, SQS, a Lambda worker, S3, and DynamoDB.

Run it from the repository root:

```bash
make pulumi-hackathon-smoke
```

Use `RUSTACK_ENDPOINT=http://127.0.0.1:4567` when port 4566 is already in use.

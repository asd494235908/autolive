FROM ghcr.io/llvm/ci-ubuntu-24.04@sha256:224c58f5d5f3f1d4b8f36dd3873b00a5d60c28065693165d875a9454ed914233

WORKDIR /work
COPY build/ /recipe/
COPY reproducible-build-lock.json /recipe/reproducible-build-lock.json
ENTRYPOINT ["/bin/bash", "/recipe/build.sh"]

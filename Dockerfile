FROM ubuntu:latest
ENV DEBIAN_FRONTEND=noninteractive
LABEL authors="rohitparab"
COPY . .
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    && rm -rf /var/lib/apt/lists/*
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo --version
ENTRYPOINT ["bash","deploy.sh"]
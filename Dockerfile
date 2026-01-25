FROM ubuntu:latest
ENV DEBIAN_FRONTEND=noninteractive

# Install dependencies + kmod for loop device support
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    kmod \
    util-linux \
    clang \
    libclang-dev \
    llvm-dev \
    iproute2 \
    && rm -rf /var/lib/apt/lists/*

COPY . /app
WORKDIR /app

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Prepare the "Template" rootfs
RUN mkdir -p /app/crater_rootfs
RUN curl -o /tmp/alpine.tar.gz https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/alpine-minirootfs-3.18.4-x86_64.tar.gz
RUN tar -xzvf /tmp/alpine.tar.gz -C /app/crater_rootfs
RUN rm /tmp/alpine.tar.gz

RUN ln -sf /bin/busybox /app/crater_rootfs/sbin/ip

# Ensure deploy script is executable
RUN chmod +x deploy.sh

ENTRYPOINT ["/app/deploy.sh"]
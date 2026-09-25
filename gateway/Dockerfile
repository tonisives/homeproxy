FROM alpine:3.22
RUN apk add --no-cache openssh-server socat tini \
    && mkdir -p /config \
    && adduser -D -s /bin/false proxy \
    && echo 'proxy:*' | chpasswd -e
COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /usr/local/bin/homeproxy-gateway
EXPOSE 2222 18081
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/homeproxy-gateway"]

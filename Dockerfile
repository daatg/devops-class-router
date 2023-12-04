FROM python:3-alpine

RUN apk add --update --no-cache ansible bash openssh cargo

# Fix ssh file perms (windows silliness)
CMD chmod 600 ~/.ssh/id_rsa

WORKDIR /usr
COPY /router /usr/router
WORKDIR /usr/router
RUN cargo build --release
WORKDIR /usr
COPY /coffee-project /usr/coffee-project
COPY hosts.yaml hosts.yaml
COPY deploy-all.yaml deploy-all.yaml
COPY deploy-router.yaml deploy-router.yaml
COPY deploy-apphosts.yaml deploy-apphosts.yaml

# CMD ansible-playbook -v -i hosts.yaml 0-update-security.yaml && \
# ansible-playbook -v -i hosts.yaml 1-setup-config.yaml && \
# ansible-playbook -v -i hosts.yaml 2-setup-config.yaml
CMD ansible-playbook -v -i hosts.yaml deploy-apphosts.yaml
# CMD ["/bin/bash"]
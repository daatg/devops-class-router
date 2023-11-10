FROM python:3-alpine

# Copy directory
COPY / /router/
WORKDIR /router

CMD ansible-playbook -v -i hosts.yaml deploy.yaml
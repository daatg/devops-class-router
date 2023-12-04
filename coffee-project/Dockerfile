FROM node:10-alpine

WORKDIR /usr/app
#RUN apk add --update nodejs npm
COPY . .
RUN npm install
EXPOSE 3000

CMD ["node", "app.js"]
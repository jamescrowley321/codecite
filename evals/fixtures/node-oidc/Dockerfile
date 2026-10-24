FROM node:26-alpine

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev

COPY provider.js ./

USER node

ENV HOST=0.0.0.0 PORT=9011
EXPOSE 9011

CMD ["node", "provider.js"]

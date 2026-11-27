FROM node:24-bookworm-slim
WORKDIR /app
COPY server.mjs picker.html ./
RUN mkdir /data && chown node:node /data
USER node
ENV PORT=8080 DATABASE=/data/groups.sqlite
EXPOSE 8080
CMD ["node","server.mjs"]

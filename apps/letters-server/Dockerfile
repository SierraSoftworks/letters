# The server as an image: the one static file and the roots it checks its provider against. CI builds it
# from a folder that holds those two (see the `server` job), and there is nothing else to run in it.
#
# The settings are read from /etc/letters/server.toml unless LETTERS_CONFIG names another file. In a
# container `listen` is set to an address its ports are published from, as `0.0.0.0:8080`.
FROM scratch
COPY ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY letters-server /letters-server
EXPOSE 8080
ENTRYPOINT ["/letters-server"]

import ssl, websocket
ws = websocket.create_connection(
    "wss://127.51.68.120:8181/",
    header=["Sec-WebSocket-Protocol: wamp"],
    sslopt={"cert_reqs": ssl.CERT_NONE}  # since it's your mkcert cert
)
print("Connected:", ws.recv())  # should print the WELCOME frame
ws.send('[5,"3dconnexion:3dcontroller/0"]')  # subscribe
print("Subscribed; streaming frames...\n")
try:
    while True:
        print(ws.recv())
except KeyboardInterrupt:
    pass

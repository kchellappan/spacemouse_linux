#!/bin/bash

mkcert -install
mkcert 127.51.68.120

echo "Make sure to import ~/.local/share/mkcert/rootCA.pem into your browser."
echo "Use it only to identify websites!"

#!/bin/bash

# Get the script directory
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# Navigate there
cd ${SCRIPT_DIR}

# Serve the web page
# Without CORS
#../spacemouse/bin/python3 -m http.server --directory ./three_js_demo/

# With Flask & CORS
../spacemouse/bin/python3 flask_server_for_three_js_demo.py

from flask import Flask, send_from_directory
from flask_cors import CORS
import os

# Get the location of this script
script_dir = os.path.dirname(os.path.realpath(__file__))

app = Flask(__name__, static_folder=os.path.join(script_dir, './three_js_demo'), static_url_path='/')

CORS(app)  # Enable CORS for all routes and all origins

@app.route('/')
def serve_index():
    # Serve the HTML file from the directory
    return send_from_directory(os.path.join(script_dir, './three_js_demo'), 'web_threejs.html')

# This will serve all static files (JS, CSS, etc.) from the 'three_js_demo' folder
@app.route('/<path:filename>')
def serve_static_files(filename):
    return send_from_directory(os.path.join(script_dir, './three_js_demo'), filename)

if __name__ == '__main__':
    app.run(port=8003)

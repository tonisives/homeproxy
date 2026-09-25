"""Disposable gateway/connector smoke test. Never uses an installed service."""
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import uuid


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kwargs).stdout.strip()


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'homeproxy-fixture')

    def log_message(self, *_args):
        pass


binary = os.environ.get('HOMEPROXY_BINARY')
if not binary:
    output = run('cargo', 'build', '-j4', '-p', 'homeproxy-cli', '--message-format=json')
    binary = next(item['executable'] for line in output.splitlines() if (item := json.loads(line)).get('executable'))
name = 'homeproxy-test-' + uuid.uuid4().hex[:10]
connector = None
with tempfile.TemporaryDirectory(prefix='.homeproxy-test-', dir=Path.cwd()) as directory:
    root = Path(directory)
    run('ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / 'identity'))
    run('ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / 'ssh_host_ed25519_key'))
    (root / 'authorized_keys').write_text((root / 'identity.pub').read_text())
    fixture = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    try:
        run('docker', 'create', '--name', name, '-p', '127.0.0.1::2222', 'homeproxy-gateway:dev')
        run('docker', 'cp', str(root) + '/.', name + ':/config')
        run('docker', 'start', name)
        ssh_port = int(run('docker', 'port', name, '2222/tcp').rsplit(':', 1)[1])
        (root / 'known_hosts').write_text(f'[127.0.0.1]:{ssh_port} ' + (root / 'ssh_host_ed25519_key.pub').read_text())
        verify_port = free_port()
        config = dict(gateway='127.0.0.1', ssh_port=ssh_port, user='proxy', identity_file=str(root / 'identity'), known_hosts_file=str(root / 'known_hosts'), remote_port=18080, verify_port=verify_port, relay_port=free_port(), mode='direct', upstream=None)
        (root / 'config.json').write_text(json.dumps(config))
        (root / 'config.json').chmod(0o600)
        time.sleep(1)
        connector = subprocess.Popen([binary, 'run'], env={**os.environ, 'HOMEPROXY_CONFIG_DIR': str(root)}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        curl = ['curl', '--fail', '--silent', '--max-time', '2', '--noproxy', '', '--proxy', f'socks5h://127.0.0.1:{verify_port}', f'http://127.0.0.1:{fixture.server_port}/']
        for _ in range(30):
            attempt = subprocess.run(curl, capture_output=True)
            if attempt.returncode == 0:
                assert attempt.stdout == b'homeproxy-fixture'
                break
            if connector.poll() is not None:
                raise RuntimeError('Connector exited before its tunnel became ready')
            time.sleep(.2)
        else:
            raise RuntimeError('Tunnel did not become ready')
        command = ['ssh', '-F', '/dev/null', '-p', str(ssh_port), '-i', str(root / 'identity'), '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes', '-o', f'UserKnownHostsFile={root / "known_hosts"}', 'proxy@127.0.0.1', 'true']
        assert subprocess.run(command, capture_output=True).returncode != 0, 'Gateway allowed shell commands'
        connector.terminate()
        connector.wait(timeout=5)
        assert subprocess.run(curl, capture_output=True).returncode != 0, 'Traffic continued after tunnel shutdown'
        print('Passed: home egress, pinned-key SSH, shell denial, shutdown and fail-closed outage')
    finally:
        if connector and connector.poll() is None:
            connector.terminate()
            connector.wait(timeout=5)
        diagnostic = subprocess.run(['docker', 'logs', name], capture_output=True, text=True)
        if diagnostic.stderr: print(diagnostic.stderr[-1500:])
        subprocess.run(['docker', 'rm', '-f', name], capture_output=True)
        fixture.shutdown()

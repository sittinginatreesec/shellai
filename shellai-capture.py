#!/usr/bin/env python3
"""Bounded, per-command output relay for the experimental Zsh integration."""
import json
import os
import re
import selectors
import shutil
import signal
import socket
import sys
import time

TAIL_BYTES = 4096


def clean(data, limit):
    text = data.decode('utf-8', errors='replace')
    text = re.sub(r'\x1b\][^\x07]*(?:\x07|\x1b\\)', '', text)
    text = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)
    text = ''.join(c for c in text if c in '\n\t' or c.isprintable())
    # An ASCII byte ceiling also bounds byte-fallback tokenization in the model.
    return text.encode('ascii', errors='replace')[-limit:].decode('ascii')


def relay(directory, parent, ready):
    os.setsid()
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    signal.signal(signal.SIGQUIT, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    selector = selectors.DefaultSelector()
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(directory + '/control')
    listener.listen(1)
    selector.register(listener, selectors.EVENT_READ, 'control')
    tails = {'stdout': b'', 'stderr': b''}
    streams = {}
    writers = []
    recording = True
    for name, output in [('stdout', 1), ('stderr', 2)]:
        path = directory + '/' + name
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
        writers.append(os.open(path, os.O_WRONLY | os.O_NONBLOCK))
        streams[fd] = (name, output)
        selector.register(fd, selectors.EVENT_READ, name)
    os.write(ready, b'1')
    os.close(ready)

    def read_stream(fd):
        nonlocal recording
        name, output = streams[fd]
        try:
            data = os.read(fd, 65536)
        except BlockingIOError:
            return False
        if not data:
            selector.unregister(fd)
            os.close(fd)
            del streams[fd]
            return False
        if recording:
            tails[name] = (tails[name] + data)[-TAIL_BYTES:]
        view = memoryview(data)
        while view:
            try:
                view = view[os.write(output, view):]
            except OSError:
                break
        return True

    try:
        while recording or streams:
            for key, _ in selector.select(0.25):
                if key.data != 'control':
                    if key.fd in streams:
                        read_stream(key.fd)
                    continue
                connection, _ = listener.accept()
                with connection:
                    # Foreground writers have closed before this request. Drain
                    # their queued bytes, but do not wait for background jobs.
                    for fd in writers:
                        os.close(fd)
                    writers.clear()
                    deadline = time.monotonic() + 0.1
                    for fd in list(streams):
                        while time.monotonic() < deadline and read_stream(fd):
                            pass
                    payload = {name: clean(data, limit) for name, data, limit in [
                        ('stdout', tails['stdout'], 256),
                        ('stderr', tails['stderr'], 512),
                    ]}
                    connection.sendall(json.dumps(payload).encode())
                recording = False
                tails.clear()
                selector.unregister(listener)
                listener.close()
                shutil.rmtree(directory, ignore_errors=True)
            try:
                os.kill(parent, 0)
            except ProcessLookupError:
                break
    finally:
        shutil.rmtree(directory, ignore_errors=True)


def main():
    mode, directory = sys.argv[1:3]
    if mode == 'start':
        for name in ('stdout', 'stderr'):
            os.mkfifo(directory + '/' + name, 0o600)
        read_fd, write_fd = os.pipe()
        pid = os.fork()
        if pid == 0:
            os.close(read_fd)
            try:
                relay(directory, int(sys.argv[3]), write_fd)
            finally:
                os._exit(0)
        os.close(write_fd)
        with os.fdopen(read_fd, 'rb') as ready:
            if ready.read(1) != b'1':
                raise RuntimeError('capture relay failed to start')
    elif mode == 'finish':
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(2)
            client.connect(directory + '/control')
            chunks = []
            while chunk := client.recv(4096):
                chunks.append(chunk)
        data = json.loads(b''.join(chunks))
        for name in ('stderr', 'stdout'):
            if data[name]:
                print(name + ' (tail):\n' + data[name])
    else:
        raise ValueError('unknown capture mode')


if __name__ == '__main__':
    main()

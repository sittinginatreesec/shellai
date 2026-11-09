"""PTY regression checks for capture, minibuffer shortcuts, and shell state.

Run with python3 tests/zsh_context.py. No model or third-party Python packages.
"""
import json
import os
from pathlib import Path
import pty
import select
import signal
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def run():
    with tempfile.TemporaryDirectory(prefix='shellai-zsh-test-') as temporary:
        directory = Path(temporary)
        stub = directory / 'shellai'
        stub.write_text('#!/usr/bin/env python3\nimport json,os,sys\n'
                        'with open(os.environ["SHELLAI_TEST_LOG"], "a") as f: '
                        'f.write(json.dumps(sys.argv[1:])+"\\n")\n'
                        'context=sys.argv[sys.argv.index("--context")+1] if "--context" in sys.argv else ""\n'
                        'print(context.splitlines()[0].split(": ",1)[1] if os.environ.get("SHELLAI_TEST_UNCHANGED") else "printf FIXED")\n')
        stub.chmod(0o755)
        log = directory / 'calls.jsonl'
        pid, fd = pty.fork()
        if pid == 0:
            os.environ.update(HOME=temporary, ZDOTDIR=temporary,
                              PATH=temporary + ':' + os.environ['PATH'],
                              SHELLAI_TEST_LOG=str(log), TERM='xterm')
            os.execvp('zsh', ['zsh', '-dfi'])
        transcript = bytearray()

        def send(text):
            os.write(fd, text.encode())

        def until(needle, timeout=5):
            pending = bytearray()
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if select.select([fd], [], [], 0.05)[0]:
                    chunk = os.read(fd, 65536)
                    transcript.extend(chunk)
                    pending.extend(chunk)
                    if needle in pending:
                        return bytes(pending)
            raise AssertionError(f'timed out waiting for {needle!r}: {bytes(transcript[-5000:])!r}')

        def command(text):
            send(text + '\n')
            return until(b'\x1b[?2004h')

        def fix():
            count = len(log.read_text().splitlines()) if log.exists() else 0
            send('\x1ba')
            until(b'Alt-F: fix last')
            send('\x1bf')
            until(b'printf FIXED')
            calls = log.read_text().splitlines()
            assert len(calls) == count + 1
            args = json.loads(calls[-1])
            assert args[-1].startswith('Fix the previous command')
            # The suggested command must remain editable, never execute.
            send('\x15\n')
            until(b'\x1b[?2004h')
            return args[args.index('--context') + 1]

        try:
            until(b'\x1b[?2004h')
            command(f"PS1='__READY__ '; RPROMPT=''; precmd() {{ :; }}; source {ROOT}/shellai.plugin.zsh")
            command("printf 'hello-out\\n'; printf 'fatal-error\\n' >&2; false")
            context = fix()
            assert 'Exit status: 1' in context, context
            assert 'hello-out' in context and 'fatal-error' in context, context
            # Repeating a fix must not replace context with Shellai's own call.
            assert fix() == context
            command(f'cd {directory}')
            assert 'Exit status: 0' in fix()
            output = command('pwd')
            assert str(directory).encode() in output
            command("python3 -c \"import sys; print('x'*100000+'OUT-END'); print('y'*100000+'ERR-END',file=sys.stderr)\"")
            context = fix()
            assert 'OUT-END' in context and 'ERR-END' in context
            assert len(context) < 1300, len(context)
            command('(sleep 1; printf BACKGROUND-DONE) &')
            context = fix()
            assert 'sleep 1' in context
            send('sleep 10\n')
            time.sleep(0.2)
            send('\x03')
            until(b'\x1b[?2004h')
            assert 'Exit status: 130' in fix()
            command('source ' + str(ROOT / 'shellai.plugin.zsh'))
            command('false')
            assert 'Exit status: 1' in fix()
            command('export SHELLAI_TEST_UNCHANGED=1')
            command('false')
            send('\x1ba')
            until(b'Alt-F: fix last')
            send('\x1bf')
            until(b'model returned the unchanged command')
            send('\x15\n')
            until(b'\x1b[?2004h')
            command('unset SHELLAI_TEST_UNCHANGED')
            # Canceling a minibuffer restores its original buffer and bindings.
            send('echo KEEP\x1ba')
            until(b'Alt-F: fix last')
            send('\x03')
            time.sleep(0.1)
            send('\x15\n')
            until(b'\x1b[?2004h')
            command('SHELLAI_CAPTURE=0')
            output = command('[[ -t 1 && -t 2 ]] && print TERMINAL-RESTORED')
            assert b'\r\nTERMINAL-RESTORED\r\n' in output, bytes(transcript[-5000:])
            print('Zsh capture and shortcut checks passed')
        finally:
            os.kill(pid, signal.SIGHUP)
            os.close(fd)
            os.waitpid(pid, 0)


if __name__ == '__main__':
    run()

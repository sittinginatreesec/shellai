"""Model-free public CLI checks."""
import subprocess
import sys
binary = sys.argv[1]
for args in (["--help"], ["ask", "--help"], ["config"]):
    subprocess.run([binary, *args], check=True, capture_output=True)
for args in (["compare", "hello"], ["--profile", "shellai", "ask", "hello"]):
    assert subprocess.run([binary, *args], capture_output=True).returncode != 0

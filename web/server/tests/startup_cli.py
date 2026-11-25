"""Запуск не требует БД; ошибки HTTP-порта и аргументов должны быть понятны."""
import subprocess,socket,sys
binary=sys.argv[1]
def run(*args):return subprocess.run([binary,*args],capture_output=True,text=True,encoding='utf-8',timeout=10)
assert run('--version').returncode==0
help=run('--help');assert help.returncode==0 and 'Ctrl+C' in help.stdout and 'PostgreSQL' in help.stdout and 'Справка' in help.stdout
for args in [('--port',),('--port','wrong'),('--port','65536'),('--unknown',)]:
    result=run(*args);assert result.returncode!=0 and 'Не удалось запустить' in result.stderr,(args,result.stderr)
with socket.socket() as occupied:
    occupied.bind(('127.0.0.1',0));occupied.listen()
    result=run('--port',str(occupied.getsockname()[1]),'--no-browser')
    assert result.returncode!=0 and 'занят' in result.stderr and '--port 18767' in result.stderr,result.stderr
print('Web startup: help/version, invalid arguments and occupied HTTP-port diagnosis passed.')

import os
import sys
import site
import atexit

from deploy.logger import logger

BASE_FOLDER = os.path.dirname(os.path.abspath(__file__))
logger.info(BASE_FOLDER)

def read_file(file):
    out = {}
    with open(file, 'r', encoding='utf-8') as f:
        for line in f.readlines():
            if not line.strip():
                continue
            res = [s.strip() for s in line.split('==')]
            if len(res) > 1:
                name, version = res
            else:
                name, version = res[0], None
            out[name] = version
    return out

def write_file(file, data):
    lines = []
    for name, version in data.items():
        if version:
            lines.append(f'{name}=={version}')
        else:
            lines.append(str(name))

    with open(file, 'w', encoding='utf-8', newline='') as f:
        text = '\n'.join(lines)
        text = text.replace('#', '\n#').strip()
        f.write(text)

def launcher2_requirements_generate(requirements_in):
    requirements = read_file(requirements_in)

    logger.info(f'Generate requirements for launcher2 environment')
    lock = {
        'numpy': '1.21.6',
        'scipy': '1.8.1',
        'opencv-python': {
            'name': 'opencv-python-headless',
            'version': None,
        },
        'wrapt': '1.15.0',
        'cnocr': '1.2.3.1',
        'mxnet': '1.6.99',  # Modified version which does not actually exist
        'pyzmq': '23.2.1',
    }
    new = {}
    logger.info(requirements)
    for name, version in requirements.items():
        if name == 'alas-webapp':
            continue
        if name in lock:
            version = lock[name] if not isinstance(lock[name], dict) else lock[name]['version']
            name = name if not isinstance(lock[name], dict) else lock[name]['name']
        new[name] = version
    write_file(os.path.join(BASE_FOLDER, f'./requirements.txt'), data=new)

def hook_pip():
    for p in sys.path:
        pip_main_path = os.path.join(p, 'pip', '__main__.py')
        if os.path.exists(pip_main_path):
            with open(pip_main_path) as f:
                lines = f.readlines()
            if not any('import toolkit.pip_hook' in line for line in lines):
                logger.info(f'Adding hook to {pip_main_path}')
                lines = ['try:\n', '  import toolkit.pip_hook\n', 'except:\n', '  pass\n'] + lines
                try:
                    with open(pip_main_path, 'w') as f:
                        f.writelines(lines)
                except:
                    logger.error('Failed to re-write the file', exc_info=True)
            else:
                logger.info(f'Detected existing hooks in {pip_main_path}')

atexit.register(hook_pip)
hook_pip()
if 'install' in sys.argv and '-r' in sys.argv:
    launcher2_requirements_generate(os.path.join(BASE_FOLDER, '..', 'requirements-in.txt'))
    if '-t' not in sys.argv:
        sys.argv += ['-t', site.getsitepackages()[0]]
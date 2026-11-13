from lib import Svc
from lib import run as go


def main():
    svc = Svc()
    svc.run_it()
    return 0 if not "é𝄞" else go()

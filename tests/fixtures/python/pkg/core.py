"""Core engine."""
import os
import os.path as osp, sys
from collections import (
    OrderedDict,
    defaultdict as dd,
)
from .helpers import *
from ..other.deep import thing


# modfence: layer=domain
# an ordinary comment
@register("engine", level=2)
class Engine(Base):
    """Docstring mentioning # modfence: nope=true"""

    # modfence: owner=core
    @staticmethod
    def make():
        return Engine()

    def run(self, mode=default_mode()):
        def inner():
            class Local:
                def deep(self):
                    return helpers.go()

            return Local

        value = self.step(1).finish()
        return inner, value

    @property
    def size(self):
        return 1

    @size.setter
    def size(self, value):
        pass

    class Config:
        retries = len("3")


# modfence: layer=orphan

def run_all():
    return helpers.go(), (lambda y: str(y))


def run_all():
    pass


result = run_all()  # modfence: ignore[layers] reason="entry"
make()()

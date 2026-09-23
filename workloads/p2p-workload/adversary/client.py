"""Access to the adversary's `Adversary` object, for the drivers and the health
checker. The object lives in the server process; method calls on the proxy
returned by `connect` run there, and exceptions they raise are re-raised here."""

from multiprocessing.managers import BaseManager

PORT = 9000
AUTHKEY = b"adversary"
# What connecting to (or calling) an unreachable adversary raises.
UNAVAILABLE = (OSError, EOFError)


class AdversaryManager(BaseManager):
    pass


AdversaryManager.register("adversary")


def connect(host="127.0.0.1"):
    """The drivers run inside the adversary container, so the default is localhost."""
    manager = AdversaryManager(address=(host, PORT), authkey=AUTHKEY)
    manager.connect()
    return manager.adversary()

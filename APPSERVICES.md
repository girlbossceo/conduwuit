# Setting up Appservices

## Getting help

If you run into any problems while setting up an Appservice, write an email to `timo@koesters.xyz`, ask us in `#conduit:matrix.org` or [open an issue on GitLab](https://gitlab.com/famedly/conduit/-/issues/new).

## Set up the appservice - general instructions

Follow whatever instructions are given by the appservice. This usually includes
downloading, changing its config (setting domain, homeserver url, port etc.)
and later starting it.

At some point the appservice guide should ask you to add a registration yaml
file to the homeserver. In Synapse you would do this by adding the path to the
homeserver.yaml, but in Conduit you can do this from within Matrix:

First, go into the #admins room of your homeserver. The first person that
registered on the homeserver automatically joins it. Then send a message into
the room like this:

    @conduit:your.server.name: register-appservice
    ```
    paste
    the
    contents
    of
    the
    yaml
    registration
    here
    ```

You can confirm it worked by sending a message like this:
`@conduit:your.server.name: list-appservices`

The @conduit bot should answer with `Appservices (1): your-bridge`

Then you are done. Conduit will send messages to the appservices and the
appservice can send requests to the homeserver. You don't need to restart
Conduit, but if it doesn't work, restarting while the appservice is running
could help.

## Appservice-specific instructions

### Remove an appservice

To remove an appservice go to your admin room and execute

```@conduit:your.server.name: unregister-appservice <name>```

where `<name>` one of the output of `list-appservices`.

### Tested appservices

These appservices have been tested and work with Conduit without any extra steps:

- [matrix-appservice-discord](https://github.com/Half-Shot/matrix-appservice-discord)
- [mautrix-hangouts](https://github.com/mautrix/hangouts/)
- [mautrix-telegram](https://github.com/mautrix/telegram/)

### [mautrix-signal](https://github.com/mautrix/signal)

There are a few things you need to do, in order for the Signal bridge (at least
up to version `0.2.0`) to work. How you do this depends on whether you use
Docker or `virtualenv` to run it. In either case you need to modify
[portal.py](https://github.com/mautrix/signal/blob/master/mautrix_signal/portal.py).
Do this **before** following the bridge installation guide.

1. **Create a copy of `portal.py`**. Go to
   [portal.py](https://github.com/mautrix/signal/blob/master/mautrix_signal/portal.py)
at [mautrix-signal](https://github.com/mautrix/signal) (make sure you change to
the correct commit/version of mautrix-signal you're using) and copy its
content. Create a new `portal.py` on your system and paste the content in.
2. **Patch the copy**. Exact line numbers may be slightly different, look nearby if they don't match: 
  - [Line 1020](https://github.com/mautrix/signal/blob/4ea831536f154aba6419d13292479eb383ea3308/mautrix_signal/portal.py#L1020)
    ```diff
    --- levels.users[self.main_intent.mxid] = 9001 if is_initial else 100
    +++ levels.users[self.main_intent.mxid] = 100 if is_initial else 100
    ```
  - [Between lines 1041 and 1042](https://github.com/mautrix/signal/blob/4ea831536f154aba6419d13292479eb383ea3308/mautrix_signal/portal.py#L1041-L1042) add a new line:
    ```diff
        "type": str(EventType.ROOM_POWER_LEVELS),
    +++ "state_key": "",
        "content": power_levels.serialize(),
    ```
3. **Deploy the patch**. This is different depending on how you have `mautrix-signal` deployed:
  - [*If using virtualenv*] Copy your patched `portal.py` to `./lib/python3.7/site-packages/mautrix_signal/portal.py` (the exact version of Python may be different on your system).
  -  [*If using Docker*] Map the patched `portal.py` into the `mautrix-signal` container:
    
    ```yaml
    volumes:
      - ./your/path/on/host/portal.py:/usr/lib/python3.9/site-packages/mautrix_signal/portal.py
    ```
4. Now continue with the [bridge installation instructions ](https://docs.mau.fi/bridges/index.html) and the general bridge notes above.

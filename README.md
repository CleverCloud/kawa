# HTX

Agnostic representation of HTTP1 and HTTP2, with zero-copy, made for Sōzu.

# Principles

Consider the following HTTP/1.1 response:

```txt
HTTP/1.1 200 OK
Transfer-Encoding: chunked
Connection: Keep-Alive
User-Agent: curl/7.43.0
Trailer: Foo

4
Wiki
5
pedia
0
Foo: bar

```

## HTX generic representation

It can be parsed in placed, extracting the essential content (header names, values...)
and stored as a vector of HTX blocks. HTX is an intermediary, protocol agnostic, representation of HTTP:

```rs
htx_blocks: [
    StatusLine::Response(V11, Slice("200"), Slice("OK")),
    Header(Slice("Transfer-Encoding"), Slice("chunked")),
    Header(Slice("Connection"), Slice("Keep-Alive")),
    Header(Slice("User-Agent"), Slice("curl/7.43.0")),
    Header(Slice("Trailer"), Slice("Foo")),
    Chunk(Slice("4\r\nWiki\r\n5\r\npedia\r\n0\r\n")),
    Header(Slice("Foo"), Slice("bar")),
]
```

## Reference buffer content with Slices

Note that `HtxBlocks` never copy data. They reference parts of the request using `Store::Slices`
which only holds a start index and a length. The request buffer can viewed as followed, marking
the referenced data in braces:

```txt
HTTP/1.1 [200] [OK]
[Transfer-Encoding]: [chunked]
[Connection]: [Keep-Alive]
[User-Agent]: [curl/7.43.0]
[Trailer]: [Foo]

[4
Wiki
5
pedia
0]
[Foo]: [bar]

```

> note: technically everything out of the braces is useless and will never be used

## HTX use cases

Say we want to:
    - remove the "User-Agent" header,
    - add a "Sozu-id" header,
    - change header "Connection" to "close",
    - change trailer "Foo" to "bazz",

All this can be accomplished regardless of the underlying protocol (HTTP/1 or HTTP/2)
using the generic HTX representation:

```rs
    htx_blocks.remove(3); // remove "User-Agent" header
    htx_blocks.insert(3, Header(Static("Sozu-id"), Vec(sozu_id.as_bytes().to_vec())));
    htx_blocks[2].val.modify("close");
    htx_blocks[6].val.modify("bazz");
```

> note: `modify` should only be used with dynamic values that will be dropped to give then a proper lifetime
> for static values (like "close") use a `Store::Static` instead, this is only for the example.
> `htx_blocks[2].val = Static("close")` would be more efficient

```rs
htx_blocks: [
    StatusLine::Response(V11, Slice("200"), Slice("OK")),
    Header(Slice("Transfer-Encoding"), Slice("chunked")),
    // "close" is shorter than "Keep-Alive" so it was written in place and kept as a Slice
    Header(Slice("Connection"), Slice("close")),
    Header(Static("Sozu-id"), Vec("SOZUBALANCEID")),
    Header(Slice("Trailer"), Slice("Foo")),
    Chunk(Slice("4\r\nWiki\r\n5\r\npedia\r\n0\r\n"))
    // "bazz" is longer than "bar" so it was dynamically allocated, this may change in the future
    Header(Slice("Foo"), Vec("bazz"))
]
```

This is what the buffer looks like now:

```txt
HTTP/1.1 [200] [OK]
[Transfer-Encoding]: [chunked]
[Connection]: [close]Alive     // "close" written in place and Slice adjusted
User-Agent: curl/7.43.0        // no references to this line
[Trailer]: [Foo]

[4
Wiki
5
pedia
0]
[Foo]: bar                     // no reference to "bar"

```

Now that the response was successfully edited we can convert it back to a specific protocol.
For simplicity's sake, let's convert it back to HTTP/1:

```rs
htx_blocks: [] // HtxBlocks are consumed
out: [
    // StatusLine::Request
    Static("HTTP/1.1"),
    Static(" "),
    Slice("200"),
    Static(" "),
    Slice("OK")
    Static("\r\n"),

    // Header
    Slice("Transfer-Encoding"),
    Static(": "),
    Slice("chunked"),
    Static("\r\n"),

    // Header
    Slice("Connection"),
    Static(": "),
    Slice("close"),
    Static("\r\n"),

    // Header
    Static("Sozu-id"),
    Static(": "),
    Vec("SOZUBALANCEID"),
    Static("\r\n"),

    // Header
    Slice("Trailer"),
    Static(": "),
    Slice("Foo"),
    Static("\r\n"),

    Static("\r\n"), // end of headers

    // Chunk
    Slice("4\r\nWiki\r\n5\r\npedia\r\n0\r\n"),

    // Header
    Slice("Foo"),
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),

    Static("\r\n"), // end of response
]
```

Every element holds data as a slice of `u8` either static, dynamic or from the response buffer.
A vector of `IoSlice` can be built from this representation and efficiently sent on a socket.
This yields the final response:

```txt
HTTP/1.1 200 OK
Transfer-Encoding: chunked
Connection: close
Sozu-id: SOZUBALANCEID
Trailer: Foo

4
Wiki
5
pedia
0
Foo: bazz

```

Say the socket only wrote up to "Wi" of "Wikipedia" (109 bytes).
In order to free this space, we can ask HTX to consume 109 bytes from its out vector.
Walking and discarding the Stores it remains:

```rs
out: [
    // <-- previous Stores were completely written so they were removed
    Slice("ki\r\n5\r\npedia\r\n0\r\n"), // Slice was partially written and updated accordingly
    Slice("Foo"),
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),
    Static("\r\n"),
]
```

Most of the data in the request buffer is not referenced anymore, and is useless now:

```txt
HTTP/1.1 200 OK
Transfer-Encoding: chunked
Connection: closeAlive
User-Agent: curl/7.43.0
Trailer: Foo

4
Wi[ki
5
pedia
0]
[Foo]: bar

```

This can be measured with `HTX::leftmost_ref` which returns the start of the leftmost Slice,
indicating that everything before that point in the buffer can be freed. Here it would return 115.
In case the user pushes left the content of the buffer, here is the buffer after the push:

```txt
ki
5
pedia
0
Foo: bar

```

As a result, the remaining Slices in the out vector reference data that has been moved.

```rs
out: [
    Slice("ki\r\n5\r\npedia\r\n0\r\n"), // references data starting at index 115
    Slice("Foo"),                       // references data starting at index 132
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),
    Static("\r\n"),
]
```

In order to synchronize the slices with the new buffer, `HTX::push_left` must be called with the
amount of bytes discarded to realigned the data:

```rs
out: [
    Slice("ki\r\n5\r\npedia\r\n0\r\n"), // references data starting at index 0
    Slice("Foo"),                       // references data starting at index 17
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),
    Static("\r\n"),
]
```

```txt
[ki
5
pedia
0]
[Foo]: bar

```
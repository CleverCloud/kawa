# Kawa

Agnostic representation of HTTP1 and HTTP2, with zero-copy, made for Sōzu.

# Principles

Consider the following HTTP/1.1 response stored in a `Buffer`:

```txt
HTTP/1.1 200 OK
Transfer-Encoding: chunked     // the body of the response is streamed
Connection: Keep-Alive
User-Agent: curl/7.43.0
Trailer: Foo                   // declares a trailer header named "Foo"

4                              // declares one chunk of 4 bytes
Wiki
5                              // declares one chunk of 5 bytes
pedia
0                              // declares one chunk of 0 byte (the last chunk)
Foo: bar                       // trailer header "Foo"

```

## HTTP generic representation

It can be parsed in place, extracting the essential content (header names, values...)
and stored as a vector of HTTP generic blocks. Kawa is an intermediary, protocol agnostic,
representation of HTTP:

```rs
kawa_blocks: [
    StatusLine::Response(V11, Slice("200"), Slice("OK")),
    Header(Slice("Transfer-Encoding"), Slice("chunked")),
    Header(Slice("Connection"), Slice("Keep-Alive")),
    Header(Slice("User-Agent"), Slice("curl/7.43.0")),
    Header(Slice("Trailer"), Slice("Foo")),
    Flags(END_HEADER),
    ChunkHeader(Slice("4")),
    Chunk(Slice("Wiki")),
    Flags(END_CHUNK),
    ChunkHeader(Slice("5")),
    Chunk(Slice("pedia")),
    Flags(END_CHUNK),
    Flags(END_BODY),
    Header(Slice("Foo"), Slice("bar")),
    Flags(END_HEADER | END_STREAM),
]
```

> note: `ChunkHeader` is the only protocol specific `Block`. It holds the chunk size present in
> an HTTP/1.1 chunk header. They can safely be ignored by an HTTP/2 converter. The `Flags` blocks
> holds context dependant information, allowing converters to be stateless.

Importantly, `Chunk` blocks don't necessarily hold an entire chunk. They may only contain a
fraction of a bigger chunk. Meaning these two representation are strictly identical:
```rs
kawa_full_chunk: [
    ChunkHeader(Slice("4")),
    Chunk(Slice("Wiki")),
    Flags(END_CHUNK),
]
kawa_fragmented_chunk: [
    ChunkHeader(Slice("4")),
    Chunk(Slice("Wi")),
    Chunk(Slice("k")),
    Chunk(Slice("i")),
    Flags(END_CHUNK),
]
```

> note: this is done in order to advance the parsing head without having to wait for potentially
> very big chunk to arrive entirely. This scheme allows more efficient streaming and prevent the
> parsers from soft locking on chunks to big to fit in their buffer.

## Reference buffer content with Slices

Note that `Blocks` never copy data. They reference parts of the request using `Store::Slices`
which only holds a start index and a length. The `Buffer` can be viewed as followed, marking
the referenced data in braces:

```txt
HTTP/1.1 [200] [OK]
[Transfer-Encoding]: [chunked]
[Connection]: [Keep-Alive]
[User-Agent]: [curl/7.43.0]
[Trailer]: [Foo]

[4]
[Wiki]
[5]
[pedia]
0
[Foo]: [bar]

```

> note: technically everything out of the braces is useless and will never be used

## Kawa use cases

Say we want to:
- remove the "User-Agent" header,
- add a "Sozu-id" header,
- change header "Connection" to "close",
- change trailer "Foo" to "bazz",

All this can be accomplished regardless of the underlying protocol (HTTP/1 or HTTP/2)
using the generic Kawa representation:

```rs
    kawa_blocks.remove(3); // remove "User-Agent" header
    kawa_blocks.insert(3, Header(Static("Sozu-id"), Vec(sozu_id.as_bytes().to_vec())));
    kawa_blocks[2].val.modify("close");
    kawa_blocks[13].val.modify("bazz");
```

> note: `modify` should only be used with dynamic values that will be dropped to give then a proper lifetime.
> For static values (like "close") use a `Store::Static` instead, this is only for the example.
> `kawa_blocks[2].val = Static("close")` would be more efficient.

```rs
kawa_blocks: [
    StatusLine::Response(V11, Slice("200"), Slice("OK")),
    Header(Slice("Transfer-Encoding"), Slice("chunked")),
    // "close" is shorter than "Keep-Alive" so it was written in place and kept as a Slice
    Header(Slice("Connection"), Slice("close")),
    Header(Static("Sozu-id"), Vec("SOZUBALANCEID")),
    Header(Slice("Trailer"), Slice("Foo")),
    Flags(END_HEADER),
    ChunkHeader(Slice("4")),
    Chunk(Slice("Wiki")),
    Flags(END_CHUNK),
    ChunkHeader(Slice("5")),
    Chunk(Slice("pedia")),
    Flags(END_CHUNK),
    Flags(END_BODY),
    // "bazz" is longer than "bar" so it was dynamically allocated, this may change in the future
    Header(Slice("Foo"), Vec("bazz"))
    Flags(END_HEADER | END_STREAM),
]
```

This is what the buffer looks like now:

```txt
HTTP/1.1 [200] [OK]
[Transfer-Encoding]: [chunked]
[Connection]: [close]Alive     // "close" written in place and Slice adjusted
User-Agent: curl/7.43.0        // no references to this line
[Trailer]: [Foo]

[4]
[Wiki]
[5]
[pedia]
0
[Foo]: bar                     // no reference to "bar"

```

Now that the response was successfully edited we can convert it back to a specific protocol.
For simplicity's sake, let's convert it back to HTTP/1:

```rs
kawa_blocks: [] // Blocks are consumed
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

    // Flags(END_HEADER)
    Static("\r\n"),

    // ChunkHeader
    Slice("4")
    Static("\r\n")
    // Chunk
    Slice("Wiki")
    // Flags(END_CHUNK)
    Static("\r\n")

    // ChunkHeader
    Slice("5")
    Static("\r\n")
    // Chunk
    Slice("pedia")
    // Flags(END_CHUNK)
    Static("\r\n")

    // Flags(END_BODY),
    Static("0\r\n")

    // Header
    Slice("Foo"),
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),

    // Flags(END_HEADER | END_STREAM)
    Static("\r\n"),
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

## Memory management

Say the socket only wrote up to "Wi" of "Wikipedia" (109 bytes).
After each write, `Kawa::consume` should be called with the number of bytes written.
This signals Kawa to free unecessary `Stores` from its `out` vector and reclaim space in its `Buffer` if possible.
In our case, Walking and discarding the `Stores` from `out` it remains:

```rs
out: [
    // <-- previous Stores were completely written so they were removed
    Slice("ki"),    // Slice was partially written and updated accordingly
    Static("\r\n"),
    Slice("5"),
    Static("\r\n"),
    Slice("pedia"),
    Static("\r\n"),
    Static("0\r\n"),
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
Wi[ki]
[5]
[pedia]
0
[Foo]: bar

```

This can be measured with `Kawa::leftmost_ref` which returns the start of the leftmost `Store::Slice`,
indicating that everything before that point in the `Buffer` is unused. Here it would return 115.
`Buffer::consume` will be called with this value. In case the `Buffer` considers that it should
shift its data to free this space (`Buffer::should_shift`), `Buffer::shift` is called memmoving
the data back to the start of the buffer. The buffer would look like:

```txt
ki
5
pedia
0
Foo: bar

```

> note: this is the only instance of copying data in this module and is necessary to not run out of
> memory unless we change the data structure of `Buffer` (with a real ring buffer for example).
> Nevertheless this should be negligeable with most shifts copying 0 or very few bytes.

As a result, the remaining `Store::Slices` in the out vector reference data that has been moved.

```rs
out: [
    Slice("ki"),    // references data starting at index 115
    Static("\r\n"),
    Slice("5"),     // references data starting at index 119
    Static("\r\n"),
    Slice("pedia"), // references...
    Static("\r\n"),
    Static("0\r\n"),
    Slice("Foo"),
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),
    Static("\r\n"),
]
```

In order to synchronize the `Store::Slices` with the new buffer, `Kawa::push_left` is called with the
amount of bytes discarded to realigned the data:

```rs
out: [
    Slice("ki"),    // references data starting at index 0
    Static("\r\n"),
    Slice("5"),     // references data starting at index 4
    Static("\r\n"),
    Slice("pedia"), // references...
    Static("\r\n"),
    Static("0\r\n"),
    Slice("Foo"),
    Static(": "),
    Vec("bazz"),
    Static("\r\n"),
    Static("\r\n"),
]
```

```txt
[ki]
[5]
[pedia]
0
[Foo]: bar

```

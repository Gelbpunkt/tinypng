# tinypng

A tiny, incomplete PNG decoder.

## Limitations

* Limited to colour types RGB and RGBA (truecolor and truecolor alpha)
* Only supports mandatory chunks (IHDR, PLTE, IDAT, IEND)

## TODO

* Implement indexed colour type to make use of PLTE
* Automated testing
* Drastically reduce amount of reallocations (this is currently awful)

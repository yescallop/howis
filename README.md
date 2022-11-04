# howis

A command-line tool to check file integrity without a checksum by redownloading.

## Help

```text
Usage: howis [OPTIONS] --src <SRC> <FILE>...

Arguments:
  <FILE>...  Files to check integrity of

Options:
  -s, --src <SRC>    Source URL list file or template string
  -r, --rec <FILE>   Record file to resume progress from [default: howis.txt]
  -u, --user <USER>  Server username
  -p, --pass <PASS>  Server password
  -h, --help         Print help information
  -V, --version      Print version information
```

Several things to clarify:

- The tool might not work properly if input filenames are identical, containing `:`, or not valid UTF-8. The URLs you put in the list file must have actual filenames as their last path segment.
- You can also use a template string as source URL, in which occurrences of `{}` will be replaced with filenames.
- Every time a downloaded file is checked, a line (e.g., `foo.zip: good`) is printed to the standard output (with average download speed) and written to the record file. A downloaded file is `good` if its content compared the same with that of the source, `bad` if not, and `error` if the source is missing or an error occurred in the request.
- After all the downloaded files are checked, the tool will attempt to fetch the undownloaded files in the URL list (if any). An undownloaded file is `n/a` if it is not available from the source (response code is not 2xx or [effective URL][1] does not contain the filename), and `error` if it is in fact available or an error occurred in the request.
- This tool cannot detect the case where a file is corrupted the same way each time you download it (e.g., truncated to a certain length due to some server defect). Ask the file provider for checksums if you're concerned about it.

[1]: https://curl.se/libcurl/c/CURLINFO_EFFECTIVE_URL.html

## License

This project is licensed under the [MIT License](/LICENSE).

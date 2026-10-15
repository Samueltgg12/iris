#!/bin/sh
# Build the TFTP test corpus in DIR (default ./shared/tftpboot), the directory
# iris serves with `tftp_dir`. Putting it under the NFS share lets the guest
# cmp what it fetched against the same file read over NFS.
set -e
DIR=${1:-shared/tftpboot}
mkdir -p "$DIR/sub"
: > "$DIR/empty"
printf x > "$DIR/one"
head -c 511  /dev/urandom > "$DIR/b511"
head -c 512  /dev/urandom > "$DIR/b512"
head -c 513  /dev/urandom > "$DIR/b513"
head -c 1024 /dev/urandom > "$DIR/b1024"
head -c 1048576 /dev/urandom > "$DIR/rand1m"
head -c 70000 /dev/urandom > "$DIR/sub/nested"
# Text for netascii: LF line ends, a bare CR, a CR at a block boundary.
{
  i=0
  while [ $i -lt 400 ]; do echo "line $i of the netascii test"; i=$((i+1)); done
  printf 'bare\rcr\n'
  head -c 511 /dev/zero | tr '\0' a; printf '\r'; printf 'tail\n'
} > "$DIR/text"
# Outside the root, for the escape tests.
echo secret > "$DIR/../tftp-outside"
echo "corpus in $DIR"

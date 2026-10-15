#!/bin/sh
# Exercise the IRIS TFTP server from a guest (IRIX sh, or any Unix).
#   sh run.sh [SERVER] [SHARE]
# SERVER defaults to the NAT gateway, SHARE (the NFS view of the TFTP root)
# to /shared/tftpboot. Build tftpget first (cc -o tftpget tftpget.c).
# Prints one PASS/FAIL line per case; exit status = number of failures.
SRV=${1:-192.168.0.1}
SHARE=${2:-/shared/tftpboot}
T=./tftpget
OUT=/tmp/tftp.out.$$
fails=0

pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; fails=`expr $fails + 1`; }

# get NAME [MODE] : fetch and compare with the NFS copy
get() {
	name=$1; mode=${2:-octet}; ref=$SHARE/`echo $name | sed 's,^/*,,'`
	rm -f $OUT
	if $T -q -m $mode $SRV $name $OUT; then
		if cmp -s $ref $OUT; then pass "$mode $name"; else fail "$mode $name (content differs)"; fi
	else
		fail "$mode $name (tftpget exit $?)"
	fi
}

# refused NAME : the server must answer with an ERROR packet
refused() {
	if $T -q -e $SRV "$1" $OUT 2>/dev/null; then pass "refused $1"; else fail "refused $1"; fi
}

for f in empty one b511 b512 b513 b1024 rand1m sub/nested text; do
	get $f
done
get /b513
get /sub/nested
get text netascii
get b1024 netascii
refused missing
refused ../tftp-outside
refused /../tftp-outside
refused sub

# The system's own client, when there is one (IRIX: /usr/bsd/tftp).
for sys in /usr/bsd/tftp /usr/bin/tftp; do
	[ -x $sys ] || continue
	rm -f $OUT
	printf 'mode binary\nget b513 %s\nquit\n' $OUT | $sys $SRV >/dev/null 2>&1
	if cmp -s $SHARE/b513 $OUT; then pass "$sys binary"; else fail "$sys binary"; fi
	rm -f $OUT
	printf 'get text %s\nquit\n' $OUT | $sys $SRV >/dev/null 2>&1
	if cmp -s $SHARE/text $OUT; then pass "$sys netascii (default mode)"; else fail "$sys netascii (default mode)"; fi
	break
done

rm -f $OUT
echo "$fails failure(s)"
exit $fails

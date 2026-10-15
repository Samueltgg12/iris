#!/bin/sh
# Guest-side network test for IRIS's NAT (IRIX sh; any Bourne shell).
#
#   sh run.sh [GROUPS...]        groups: local udp ext ftp telnet stall all
#
# Needs ./nettest (cc -o nettest nettest.c) and, for "local"/"udp", the host
# running `nettest serve 15555` (reached at the gateway, which the NAT maps to
# host loopback). External targets come from the environment:
#   HTTP_HOST (nosuchaddress.net)  FTP_HOST FTP_USER FTP_PASS  TELNET_HOST
#   TELNET_USER TELNET_PASS
# One PASS/FAIL/SKIP line per case; exit status = number of failures.
GW=${GW:-192.168.0.1}
PORT=${PORT:-15555}
HTTP_HOST=${HTTP_HOST:-nosuchaddress.net}
N=./nettest
fails=0

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; fails=`expr $fails + 1`; }
skip() { echo "SKIP $*"; }

# t NAME CMD... : run a nettest client, keep its result line
t() {
	name=$1; shift
	out=`"$@" 2>&1 | grep '^nettest:' | tail -1`
	res=`echo "$out" | sed 's/^nettest: [A-Z]* //'`
	case "$out" in
	"nettest: PASS"*) pass "$name: $res" ;;
	*) fail "$name: ${res:-no output}" ;;
	esac
}

g_local() {
	t "tcp sink 64K"     $N sink   $GW $PORT 65536
	t "tcp sink 4M"      $N sink   $GW $PORT 4194304
	t "tcp source 64K"   $N source $GW $PORT 65536
	t "tcp source 4M"    $N source $GW $PORT 4194304
	t "tcp echo 1M"      $N echo   $GW $PORT 1048576
	t "tcp connect"      $N connect $GW $PORT
}

g_udp() {
	t "udp 200x64"       $N udp $GW $PORT 200 64
	t "udp 200x1400"     $N udp $GW $PORT 200 1400
	t "udp 50x4000 frag" $N udp $GW $PORT 50 4000
	t "udp 20x30000 frag" $N udp $GW $PORT 20 30000
}

g_ext() {
	t "http $HTTP_HOST"  $N http $HTTP_HOST 80 /
	ping -c 3 $HTTP_HOST > /tmp/nt.ping 2>&1
	if egrep ' 0% packet loss| 0.0% packet loss' /tmp/nt.ping > /dev/null; then
		pass "ping $HTTP_HOST"
	else
		fail "ping $HTTP_HOST: `tail -2 /tmp/nt.ping | tr '\n' ' '`"
	fi
}

# ftp_xfer LABEL : put a 1 MB file and get it back
ftp_xfer() {
	mode=$1
	[ -n "$FTP_HOST" ] || { skip "ftp $mode (FTP_HOST unset)"; return; }
	rm -f /tmp/nt.ftpback
	(
		echo "user $FTP_USER $FTP_PASS"
		echo "binary"
		echo "cd test"
		echo "put /tmp/nt.ftp1m nt-$$.bin"
		echo "get nt-$$.bin /tmp/nt.ftpback"
		echo "delete nt-$$.bin"
		echo "quit"
	) | ftp -n -v $FTP_HOST > /tmp/nt.ftplog 2>&1
	if cmp -s /tmp/nt.ftp1m /tmp/nt.ftpback; then
		rate=`egrep 'bytes sent|bytes received' /tmp/nt.ftplog | sed 's/.*(\(.*\))/\1/' | tr '\n' ' '`
		pass "ftp $mode 1M round trip ($rate)"
	else
		fail "ftp $mode: `egrep '^[45][0-9][0-9] |refused|timed out|not connected|Invalid' /tmp/nt.ftplog | head -2 | tr '\n' ' '`"
	fi
}

g_ftp() {
	dd if=/unix of=/tmp/nt.ftp1m bs=1024 count=1024 > /dev/null 2>&1
	# IRIX ftp always uses EPSV/PASV (no "passive" toggle), so this is the
	# passive path: control and data connections both go out through the NAT.
	ftp_xfer epsv
}

g_telnet() {
	[ -n "$TELNET_HOST" ] || { skip "telnet (TELNET_HOST unset)"; return; }
	( sleep 4; echo "$TELNET_USER"; sleep 3; echo "$TELNET_PASS"; sleep 4;
	  echo "echo NT-MARK-\`uname -s\`"; sleep 3; echo exit; sleep 1 ) |
		telnet $TELNET_HOST > /tmp/nt.telnet 2>&1
	if grep 'NT-MARK-IRIX' /tmp/nt.telnet > /dev/null; then
		pass "telnet $TELNET_HOST login + command"
	else
		fail "telnet $TELNET_HOST: `tail -3 /tmp/nt.telnet | tr '\n' ' '`"
	fi
}

# A connect to a black-holed address must not stall everyone else's traffic.
g_stall() {
	$N connect 10.255.255.1 80 > /dev/null 2>&1 &
	sleep 1
	ping -c 4 $GW > /tmp/nt.ping 2>&1
	max=`grep 'round-trip' /tmp/nt.ping | sed 's,.*= *[0-9.]*/[0-9.]*/\([0-9]*\).*,\1,'`
	if [ -n "$max" ] && [ "$max" -lt 500 ]; then
		pass "no stall during black-holed connect (ping max ${max} ms)"
	else
		fail "NAT stalled during black-holed connect (ping max ${max:-?} ms)"
	fi
	kill $! 2>/dev/null
}

groups=${*:-all}
[ "$groups" = all ] && groups="local udp ext ftp telnet stall"
for g in $groups; do
	g_$g
done
echo "$fails failure(s)"
exit $fails

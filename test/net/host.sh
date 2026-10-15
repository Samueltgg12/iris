#!/bin/sh
# Host side of the network tests: port forwards into the guest.
#   sh test/net/host.sh [NETTEST]
# Expects the guest to run `nettest serve 15555` and iris to forward host
# 127.0.0.1:15600 (tcp and udp) to guest 15555:
#   [[port_forward]] proto="tcp" host_port=15600 guest_port=15555 bind="localhost"
#   [[port_forward]] proto="udp" host_port=15600 guest_port=15555 bind="localhost"
N=${1:-./nettest}
H=127.0.0.1
P=${FWD_PORT:-15600}
fails=0
t() {
	name=$1; shift
	out=`"$@" 2>&1 | grep '^nettest:' | tail -1`
	case "$out" in
	"nettest: PASS"*) echo "PASS fwd $name: ${out#nettest: PASS }" ;;
	*) echo "FAIL fwd $name: ${out:-no output}"; fails=$((fails+1)) ;;
	esac
}
t "tcp connect"    $N connect $H $P
t "tcp sink 4M"    $N sink    $H $P 4194304
t "tcp source 4M"  $N source  $H $P 4194304
t "tcp echo 1M"    $N echo    $H $P 1048576
t "udp 200x1400"   $N udp     $H $P 200 1400
t "udp 20x8000 frag" $N udp   $H $P 20 8000
echo "$fails failure(s)"
exit $fails

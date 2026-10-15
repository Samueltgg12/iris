/*
 * nettest - directed TCP/UDP tests for IRIS's NAT, from either end.
 *
 * One program for the guest (IRIX 5.3/6.5, MIPSpro cc) and the host (Linux,
 * macOS, gcc/clang): plain C89 and BSD sockets.
 *     cc -o nettest nettest.c                 (IRIX)
 *     gcc -O2 -o nettest nettest.c            (Linux/macOS)
 *
 * Server (TCP and UDP on the same port, forks per TCP connection):
 *     nettest serve PORT
 *
 * Clients (HOST may be a name or dotted quad):
 *     nettest sink   HOST PORT BYTES   send BYTES, server verifies and reports
 *     nettest source HOST PORT BYTES   server sends BYTES, we verify
 *     nettest echo   HOST PORT BYTES   send BYTES, verify the echo (full duplex)
 *     nettest udp    HOST PORT COUNT SIZE   COUNT datagrams of SIZE, echoed back
 *     nettest http   HOST PORT PATH    GET PATH, print the status line and size
 *     nettest connect HOST PORT        connect only; prints the time it took
 *
 * TCP protocol: the client's first line picks the test ("SINK n\n",
 * "SOURCE n\n", "ECHO n\n"). SINK ends with the server's "OK n sum\n".
 * Data is the pattern byte(i) = (i % 251), so any lost, duplicated or
 * reordered byte shows up at its offset.
 *
 * Every client prints one result line:
 *     nettest: PASS <mode> <bytes> bytes <ms> ms <KB/s> KB/s [extra]
 *     nettest: FAIL <mode> <reason>
 * and exits 0 on PASS, 1 on FAIL, 2 on usage or setup errors.
 */
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#if defined(__sgi) && !defined(_SOCKLEN_T)
typedef int nt_socklen;
#else
typedef socklen_t nt_socklen;
#endif

#define BUFSZ 16384
#define IO_TIMEOUT_S 20

static char *mode_name = "";

static long
now_ms(void)
{
	struct timeval tv;
	gettimeofday(&tv, (void *)0);
	return (long)tv.tv_sec * 1000L + tv.tv_usec / 1000;
}

static int
pat(long i)
{
	return (int)(i % 251);
}

/*
 * The pattern from offset o is ptable[o % 251 ...]: one table holding the
 * period plus a buffer's worth, so fills and checks are memcpy/memcmp
 * instead of a division per byte (which dominates on an emulated CPU).
 */
static unsigned char ptable[251 + BUFSZ];

static void
init_ptable(void)
{
	int i;
	for (i = 0; i < (int)sizeof(ptable); i++)
		ptable[i] = (unsigned char)(i % 251);
}

/* Offset of the first byte of buf[0..n) that breaks the pattern, or -1. */
static long
check_pattern(const char *buf, long start, int n)
{
	int i;
	if (memcmp(buf, ptable + start % 251, n) == 0)
		return -1;
	for (i = 0; i < n; i++)
		if ((unsigned char)buf[i] != pat(start + i))
			return start + i;
	return -1;
}

static void
fail(const char *fmt, const char *arg, long n)
{
	fprintf(stderr, "nettest: FAIL %s ", mode_name);
	fprintf(stderr, fmt, arg, n);
	fprintf(stderr, "\n");
	exit(1);
}

static void
pass(long bytes, long ms, const char *extra)
{
	long rate = ms > 0 ? bytes / ms : bytes; /* bytes per ms == KB/s (1000) */
	fprintf(stderr, "nettest: PASS %s %ld bytes %ld ms %ld KB/s%s%s\n",
	    mode_name, bytes, ms, rate, extra[0] ? " " : "", extra);
	exit(0);
}

static void
resolve(const char *host, int port, struct sockaddr_in *sa)
{
	struct hostent *he;

	memset(sa, 0, sizeof(*sa));
	sa->sin_family = AF_INET;
	sa->sin_port = htons((unsigned short)port);
	sa->sin_addr.s_addr = inet_addr(host);
	if (sa->sin_addr.s_addr == htonl(0xffffffffUL)) {
		if ((he = gethostbyname(host)) == (struct hostent *)0) {
			fprintf(stderr, "nettest: unknown host %s\n", host);
			exit(2);
		}
		memcpy(&sa->sin_addr, he->h_addr, sizeof(sa->sin_addr));
	}
}

/* Wait until fd is readable (want_write=0) or writable; 0 on timeout. */
static int
wait_fd(int fd, int want_write, int timeout_s)
{
	fd_set fds;
	struct timeval tv;
	int n;

	for (;;) {
		FD_ZERO(&fds);
		FD_SET(fd, &fds);
		tv.tv_sec = timeout_s;
		tv.tv_usec = 0;
		n = select(fd + 1, want_write ? (fd_set *)0 : &fds,
		    want_write ? &fds : (fd_set *)0, (fd_set *)0, &tv);
		if (n < 0 && errno == EINTR)
			continue;
		return n > 0;
	}
}

static int
tcp_connect(const char *host, int port)
{
	struct sockaddr_in sa;
	int s;

	resolve(host, port, &sa);
	if ((s = socket(AF_INET, SOCK_STREAM, 0)) < 0) {
		perror("nettest: socket");
		exit(2);
	}
	if (connect(s, (struct sockaddr *)&sa, sizeof(sa)) < 0)
		fail("connect: %s%ld", strerror(errno), 0L);
	return s;
}

/* Write all of buf, failing on a stall longer than IO_TIMEOUT_S. */
static void
write_all(int s, const char *buf, long n)
{
	long off = 0;
	int w;

	while (off < n) {
		if (!wait_fd(s, 1, IO_TIMEOUT_S))
			fail("write stalled at %s%ld", "offset ", off);
		w = write(s, buf + off, (int)(n - off));
		if (w < 0) {
			if (errno == EINTR)
				continue;
			fail("write: %s (%ld written)", strerror(errno), off);
		}
		off += w;
	}
}

/* Read one '\n'-terminated line (without the newline) into buf. */
static int
read_line(int s, char *buf, int size)
{
	int n = 0, r;
	char c;

	while (n < size - 1) {
		if (!wait_fd(s, 0, IO_TIMEOUT_S))
			return -1;
		r = read(s, &c, 1);
		if (r <= 0)
			return -1;
		if (c == '\n')
			break;
		buf[n++] = c;
	}
	buf[n] = 0;
	return n;
}

static void
fill_pattern(char *buf, long start, int n)
{
	memcpy(buf, ptable + start % 251, n);
}

/* Read exactly `want` pattern bytes starting at stream offset 0, then EOF. */
static long
read_verify(int s, long want, int expect_eof)
{
	static char buf[BUFSZ];
	long got = 0, bad;
	int r;

	for (;;) {
		if (!wait_fd(s, 0, IO_TIMEOUT_S))
			fail("read stalled at offset %s%ld", "", got);
		r = read(s, buf, sizeof(buf));
		if (r < 0) {
			if (errno == EINTR)
				continue;
			fail("read: %s (%ld bytes so far)", strerror(errno), got);
		}
		if (r == 0)
			break;
		if ((bad = check_pattern(buf, got, r)) >= 0)
			fail("bad byte at offset %s%ld", "", bad);
		got += r;
		if (!expect_eof && got >= want)
			break;
	}
	if (got != want)
		fail("short stream: got %s%ld", "", got);
	return got;
}

static void
send_pattern(int s, long bytes)
{
	static char buf[BUFSZ];
	long off = 0;
	int n;

	while (off < bytes) {
		n = bytes - off > BUFSZ ? BUFSZ : (int)(bytes - off);
		fill_pattern(buf, off, n);
		write_all(s, buf, n);
		off += n;
	}
}

/* ── clients ─────────────────────────────────────────────────────────── */

static void
client_sink(const char *host, int port, long bytes)
{
	char line[128];
	long t0, got = -1;
	int s;

	s = tcp_connect(host, port);
	t0 = now_ms();
	sprintf(line, "SINK %ld\n", bytes);
	write_all(s, line, (long)strlen(line));
	send_pattern(s, bytes);
	shutdown(s, 1);
	if (read_line(s, line, sizeof(line)) < 0)
		fail("no verdict from server%s%ld", "", 0L);
	if (sscanf(line, "OK %ld", &got) != 1 || got != bytes)
		fail("server says: %s%ld", line, 0L);
	close(s);
	pass(bytes, now_ms() - t0, "");
}

static void
client_source(const char *host, int port, long bytes)
{
	char line[64];
	long t0;
	int s;

	s = tcp_connect(host, port);
	t0 = now_ms();
	sprintf(line, "SOURCE %ld\n", bytes);
	write_all(s, line, (long)strlen(line));
	read_verify(s, bytes, 1);
	close(s);
	pass(bytes, now_ms() - t0, "");
}

/* Full duplex: interleave writes and reads so neither side's buffer fills. */
static void
client_echo(const char *host, int port, long bytes)
{
	static char wbuf[BUFSZ], rbuf[BUFSZ];
	char line[64];
	long t0, sent = 0, got = 0, bad;
	fd_set rfds, wfds;
	struct timeval tv;
	int s, n;

	s = tcp_connect(host, port);
	t0 = now_ms();
	sprintf(line, "ECHO %ld\n", bytes);
	write_all(s, line, (long)strlen(line));
	while (got < bytes) {
		FD_ZERO(&rfds);
		FD_ZERO(&wfds);
		FD_SET(s, &rfds);
		if (sent < bytes)
			FD_SET(s, &wfds);
		tv.tv_sec = IO_TIMEOUT_S;
		tv.tv_usec = 0;
		n = select(s + 1, &rfds, &wfds, (fd_set *)0, &tv);
		if (n < 0 && errno == EINTR)
			continue;
		if (n <= 0)
			fail("echo stalled: received %s%ld", "", got);
		if (FD_ISSET(s, &wfds)) {
			n = bytes - sent > BUFSZ ? BUFSZ : (int)(bytes - sent);
			fill_pattern(wbuf, sent, n);
			n = write(s, wbuf, n);
			if (n < 0 && errno != EINTR && errno != EAGAIN)
				fail("write: %s (%ld sent)", strerror(errno), sent);
			if (n > 0)
				sent += n;
		}
		if (FD_ISSET(s, &rfds)) {
			n = read(s, rbuf, sizeof(rbuf));
			if (n <= 0)
				fail("echo closed early: received %s%ld", "", got);
			if ((bad = check_pattern(rbuf, got, n)) >= 0)
				fail("bad echo byte at offset %s%ld", "", bad);
			got += n;
		}
	}
	close(s);
	pass(bytes, now_ms() - t0, "");
}

static void
client_udp(const char *host, int port, long count, int size)
{
	static char wbuf[65536], rbuf[65536];
	struct sockaddr_in sa, from;
	nt_socklen fromlen;
	long i, ok = 0, lost = 0, t0, t, rtt_sum = 0, rtt_max = 0;
	char extra[96];
	int s, r, j;

	if (size < 8 || size > 65000) {
		fprintf(stderr, "nettest: udp size must be 8..65000\n");
		exit(2);
	}
	resolve(host, port, &sa);
	if ((s = socket(AF_INET, SOCK_DGRAM, 0)) < 0) {
		perror("nettest: socket");
		exit(2);
	}
	t0 = now_ms();
	for (i = 0; i < count; i++) {
		/* first 8 bytes: sequence number, rest: pattern */
		sprintf(wbuf, "%07ld", i % 10000000L);
		for (j = 8; j < size; j++)
			wbuf[j] = (char)pat(i + j);
		t = now_ms();
		if (sendto(s, wbuf, size, 0, (struct sockaddr *)&sa, sizeof(sa)) != size)
			fail("sendto: %s%ld", strerror(errno), 0L);
		for (;;) {
			if (!wait_fd(s, 0, 1)) {
				lost++;
				break;
			}
			fromlen = sizeof(from);
			r = recvfrom(s, rbuf, sizeof(rbuf), 0, (struct sockaddr *)&from, &fromlen);
			if (r < 0)
				fail("recvfrom: %s%ld", strerror(errno), 0L);
			if (r != size || memcmp(rbuf, wbuf, size) != 0) {
				if (r >= 8 && memcmp(rbuf, wbuf, 7) != 0)
					continue;	/* a late reply to an earlier datagram */
				fail("bad echo for datagram %s%ld", "", i);
			}
			ok++;
			t = now_ms() - t;
			rtt_sum += t;
			if (t > rtt_max)
				rtt_max = t;
			break;
		}
	}
	sprintf(extra, "ok %ld lost %ld rtt_avg %ld ms rtt_max %ld ms",
	    ok, lost, ok ? rtt_sum / ok : 0L, rtt_max);
	if (lost > count / 100)		/* tolerate 1% on a real network */
		fail("%s%ld", extra, 0L);
	pass(ok * size, now_ms() - t0, extra);
}

static void
client_http(const char *host, int port, const char *path)
{
	static char buf[BUFSZ];
	char req[512], status[256], extra[300];
	long t0, total = 0;
	int s, r, n = 0, got_status = 0;

	if (strlen(path) + strlen(host) > 400) {
		fprintf(stderr, "nettest: path too long\n");
		exit(2);
	}
	s = tcp_connect(host, port);
	t0 = now_ms();
	sprintf(req, "GET %s HTTP/1.0\r\nHost: %s\r\nUser-Agent: nettest\r\n\r\n", path, host);
	write_all(s, req, (long)strlen(req));
	for (;;) {
		if (!wait_fd(s, 0, IO_TIMEOUT_S))
			fail("http read stalled after %s%ld bytes", "", total);
		r = read(s, buf, sizeof(buf));
		if (r < 0 && errno == EINTR)
			continue;
		if (r <= 0)
			break;
		if (!got_status) {
			int i;
			for (i = 0; i < r && n < (int)sizeof(status) - 1; i++) {
				if (buf[i] == '\r' || buf[i] == '\n') {
					got_status = 1;
					break;
				}
				status[n++] = buf[i];
			}
			status[n] = 0;
		}
		total += r;
	}
	close(s);
	if (strncmp(status, "HTTP/", 5) != 0)
		fail("no HTTP status line (%s)%ld", status, 0L);
	sprintf(extra, "\"%.200s\"", status);
	pass(total, now_ms() - t0, extra);
}

static void
client_connect(const char *host, int port)
{
	long t0 = now_ms();
	int s = tcp_connect(host, port);
	close(s);
	pass(0L, now_ms() - t0, "");
}

/* ── server ──────────────────────────────────────────────────────────── */

static long
adler_add(long sum, const char *p, int n)
{
	unsigned long a = sum & 0xffff, b = (sum >> 16) & 0xffff;
	int i, k;
	/* Reduce every 5552 bytes, the most that cannot overflow 32 bits (zlib). */
	while (n > 0) {
		k = n < 5552 ? n : 5552;
		for (i = 0; i < k; i++) {
			a += (unsigned char)p[i];
			b += a;
		}
		a %= 65521UL;
		b %= 65521UL;
		p += k;
		n -= k;
	}
	return (long)((b << 16) | a);
}

static void
serve_conn(int c)
{
	static char buf[BUFSZ];
	char line[128];
	long n = 0, got = 0, sum = 1, bad = -1;
	int r;

	if (read_line(c, line, sizeof(line)) < 0)
		_exit(1);
	if (sscanf(line, "SINK %ld", &n) == 1) {
		for (;;) {
			if (!wait_fd(c, 0, IO_TIMEOUT_S))
				break;
			r = read(c, buf, sizeof(buf));
			if (r < 0 && errno == EINTR)
				continue;
			if (r <= 0)
				break;
			if (bad < 0)
				bad = check_pattern(buf, got, r);
			sum = adler_add(sum, buf, r);
			got += r;
		}
		if (bad >= 0)
			sprintf(line, "BAD byte at %ld of %ld\n", bad, got);
		else if (got != n)
			sprintf(line, "SHORT %ld of %ld\n", got, n);
		else
			sprintf(line, "OK %ld %08lx\n", got, sum);
		write_all(c, line, (long)strlen(line));
	} else if (sscanf(line, "SOURCE %ld", &n) == 1) {
		send_pattern(c, n);
	} else if (sscanf(line, "ECHO %ld", &n) == 1) {
		while (got < n) {
			if (!wait_fd(c, 0, IO_TIMEOUT_S))
				break;
			r = read(c, buf, sizeof(buf));
			if (r < 0 && errno == EINTR)
				continue;
			if (r <= 0)
				break;
			write_all(c, buf, r);
			got += r;
		}
	}
	close(c);
	_exit(0);
}

static void
serve_udp(int port)
{
	static char buf[65536];
	struct sockaddr_in sa, from;
	nt_socklen fromlen;
	int s, r, on = 1;

	if ((s = socket(AF_INET, SOCK_DGRAM, 0)) < 0) {
		perror("nettest: udp socket");
		_exit(2);
	}
	setsockopt(s, SOL_SOCKET, SO_REUSEADDR, (char *)&on, sizeof(on));
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons((unsigned short)port);
	if (bind(s, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		perror("nettest: udp bind");
		_exit(2);
	}
	for (;;) {
		fromlen = sizeof(from);
		r = recvfrom(s, buf, sizeof(buf), 0, (struct sockaddr *)&from, &fromlen);
		if (r > 0)
			sendto(s, buf, r, 0, (struct sockaddr *)&from, fromlen);
	}
}

static void
reap(int sig)
{
	(void)sig;
	while (waitpid(-1, (int *)0, WNOHANG) > 0)
		;
	signal(SIGCHLD, reap);
}

static void
serve(int port)
{
	struct sockaddr_in sa;
	int s, c, on = 1;

	signal(SIGCHLD, reap);
	signal(SIGPIPE, SIG_IGN);
	if (fork() == 0)
		serve_udp(port);
	if ((s = socket(AF_INET, SOCK_STREAM, 0)) < 0) {
		perror("nettest: socket");
		exit(2);
	}
	setsockopt(s, SOL_SOCKET, SO_REUSEADDR, (char *)&on, sizeof(on));
	memset(&sa, 0, sizeof(sa));
	sa.sin_family = AF_INET;
	sa.sin_port = htons((unsigned short)port);
	if (bind(s, (struct sockaddr *)&sa, sizeof(sa)) < 0 || listen(s, 16) < 0) {
		perror("nettest: bind/listen");
		exit(2);
	}
	fprintf(stderr, "nettest: serving tcp+udp on %d\n", port);
	for (;;) {
		c = accept(s, (struct sockaddr *)0, (nt_socklen *)0);
		if (c < 0)
			continue;
		if (fork() == 0) {
			close(s);
			serve_conn(c);
		}
		close(c);
	}
}

static void
usage(void)
{
	fprintf(stderr,
	    "usage: nettest serve PORT\n"
	    "       nettest sink|source|echo HOST PORT BYTES\n"
	    "       nettest udp HOST PORT COUNT SIZE\n"
	    "       nettest http HOST PORT PATH\n"
	    "       nettest connect HOST PORT\n");
	exit(2);
}

int
main(int argc, char **argv)
{
	if (argc < 3)
		usage();
	mode_name = argv[1];
	signal(SIGPIPE, SIG_IGN);
	init_ptable();
	if (strcmp(argv[1], "serve") == 0)
		serve(atoi(argv[2]));
	else if (argc == 5 && strcmp(argv[1], "sink") == 0)
		client_sink(argv[2], atoi(argv[3]), atol(argv[4]));
	else if (argc == 5 && strcmp(argv[1], "source") == 0)
		client_source(argv[2], atoi(argv[3]), atol(argv[4]));
	else if (argc == 5 && strcmp(argv[1], "echo") == 0)
		client_echo(argv[2], atoi(argv[3]), atol(argv[4]));
	else if (argc == 6 && strcmp(argv[1], "udp") == 0)
		client_udp(argv[2], atoi(argv[3]), atol(argv[4]), atoi(argv[5]));
	else if (argc == 5 && strcmp(argv[1], "http") == 0)
		client_http(argv[2], atoi(argv[3]), argv[4]);
	else if (argc == 4 && strcmp(argv[1], "connect") == 0)
		client_connect(argv[2], atoi(argv[3]));
	else
		usage();
	return 0;
}

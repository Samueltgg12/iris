/*
 * tftpget - minimal RFC 1350 TFTP read client, for testing IRIS's built-in
 * TFTP server from inside a guest (IRIX 5.3/6.5) or from a Linux/BSD host.
 *
 * Plain C89 + BSD sockets so MIPSpro cc, IRIX gcc and Linux gcc all build it:
 *     cc -o tftpget tftpget.c                (IRIX)
 *     gcc -O2 -Wall -o tftpget tftpget.c     (Linux)
 *
 * usage: tftpget [-m octet|netascii] [-p port] [-t timeout_s] [-r retries]
 *                [-e] [-q] host remote-file [local-file]
 *
 *   -e  expect an ERROR packet (negative tests); exit 0 only if one arrives.
 *   -q  quiet: print only the summary line.
 *
 * Without local-file the data goes to stdout. The summary line on stderr is
 *     tftpget: OK <bytes> bytes <blocks> blocks <retries> retries <ms> ms sum <hex>
 * or  tftpget: ERROR <code> <message>
 * where sum is a 32-bit Adler-style checksum of the decoded bytes.
 *
 * Exit status: 0 success, 1 TFTP error packet, 2 timeout, 3 protocol
 * violation (wrong block, bad opcode), 4 usage / local I/O.
 */
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define SEGSIZE   512
#define PKTSIZE   (SEGSIZE + 4)

#define OP_RRQ    1
#define OP_DATA   3
#define OP_ACK    4
#define OP_ERROR  5

static int quiet;
static unsigned long sum_a = 1, sum_b = 0;
static long total_bytes;
/* netascii decode state: a CR is held until the next byte says what it was */
static int pending_cr;

static void
usage(void)
{
	fprintf(stderr, "usage: tftpget [-m octet|netascii] [-p port] [-t timeout_s] "
	    "[-r retries] [-e] [-q] host remote-file [local-file]\n");
	exit(4);
}

static long
now_ms(void)
{
	struct timeval tv;
	gettimeofday(&tv, (void *)0);
	return (long)tv.tv_sec * 1000L + tv.tv_usec / 1000;
}

static void
put_byte(FILE *out, int c)
{
	putc(c, out);
	sum_a = (sum_a + (unsigned char)c) % 65521UL;
	sum_b = (sum_b + sum_a) % 65521UL;
	total_bytes++;
}

/* RFC 764: CR LF -> LF, CR NUL -> CR; any other CR x is passed through. */
static void
put_data(FILE *out, const unsigned char *p, int n, int netascii)
{
	int i;

	for (i = 0; i < n; i++) {
		int c = p[i];
		if (!netascii) {
			put_byte(out, c);
			continue;
		}
		if (pending_cr) {
			pending_cr = 0;
			if (c == '\n') {
				put_byte(out, '\n');
				continue;
			}
			put_byte(out, '\r');
			if (c == '\0')
				continue;
		}
		if (c == '\r')
			pending_cr = 1;
		else
			put_byte(out, c);
	}
}

static int
send_pkt(int s, struct sockaddr_in *to, const unsigned char *p, int n)
{
	if (sendto(s, (const char *)p, n, 0, (struct sockaddr *)to, sizeof(*to)) != n) {
		perror("tftpget: sendto");
		return -1;
	}
	return 0;
}

/* Wait up to timeout_s for a datagram; returns length, 0 on timeout, -1 on error. */
static int
recv_pkt(int s, unsigned char *buf, int size, struct sockaddr_in *from, int timeout_s)
{
	fd_set fds;
	struct timeval tv;
	int n;
#if defined(__sgi) && !defined(_SOCKLEN_T)
	int fromlen;
#else
	socklen_t fromlen;
#endif

	FD_ZERO(&fds);
	FD_SET(s, &fds);
	tv.tv_sec = timeout_s;
	tv.tv_usec = 0;
	n = select(s + 1, &fds, (fd_set *)0, (fd_set *)0, &tv);
	if (n < 0) {
		if (errno == EINTR)
			return 0;
		perror("tftpget: select");
		return -1;
	}
	if (n == 0)
		return 0;
	fromlen = sizeof(*from);
	n = recvfrom(s, (char *)buf, size, 0, (struct sockaddr *)from, &fromlen);
	if (n < 0) {
		perror("tftpget: recvfrom");
		return -1;
	}
	return n;
}

static unsigned
get16(const unsigned char *p)
{
	return ((unsigned)p[0] << 8) | p[1];
}

static void
put16(unsigned char *p, unsigned v)
{
	p[0] = (unsigned char)(v >> 8);
	p[1] = (unsigned char)v;
}

int
main(int argc, char **argv)
{
	const char *mode = "octet";
	int port = 69, timeout_s = 2, max_retries = 5, expect_error = 0;
	const char *host, *remote, *local = (const char *)0;
	struct sockaddr_in server, from, bind_addr;
	struct hostent *he;
	unsigned char req[600], ack[4], buf[PKTSIZE + 64];
	int s, n, reqlen, netascii, retries = 0, total_retries = 0;
	int have_tid = 0;
	unsigned expect_block = 1;
	long blocks = 0, t0;
	FILE *out;
	int c;

	while ((c = getopt(argc, argv, "m:p:t:r:eq")) != -1) {
		switch (c) {
		case 'm': mode = optarg; break;
		case 'p': port = atoi(optarg); break;
		case 't': timeout_s = atoi(optarg); break;
		case 'r': max_retries = atoi(optarg); break;
		case 'e': expect_error = 1; break;
		case 'q': quiet = 1; break;
		default: usage();
		}
	}
	if (argc - optind < 2 || argc - optind > 3)
		usage();
	host = argv[optind];
	remote = argv[optind + 1];
	if (argc - optind == 3)
		local = argv[optind + 2];
	netascii = strcmp(mode, "netascii") == 0;
	if (!netascii && strcmp(mode, "octet") != 0)
		usage();
	if (strlen(remote) + strlen(mode) + 4 > sizeof(req)) {
		fprintf(stderr, "tftpget: file name too long\n");
		return 4;
	}

	memset(&server, 0, sizeof(server));
	server.sin_family = AF_INET;
	server.sin_port = htons((unsigned short)port);
	server.sin_addr.s_addr = inet_addr(host);
	if (server.sin_addr.s_addr == htonl(0xffffffffUL)) {
		if ((he = gethostbyname(host)) == (struct hostent *)0) {
			fprintf(stderr, "tftpget: unknown host %s\n", host);
			return 4;
		}
		memcpy(&server.sin_addr, he->h_addr, sizeof(server.sin_addr));
	}

	if ((s = socket(AF_INET, SOCK_DGRAM, 0)) < 0) {
		perror("tftpget: socket");
		return 4;
	}
	memset(&bind_addr, 0, sizeof(bind_addr));
	bind_addr.sin_family = AF_INET;
	if (bind(s, (struct sockaddr *)&bind_addr, sizeof(bind_addr)) < 0) {
		perror("tftpget: bind");
		return 4;
	}

	if (local) {
		if ((out = fopen(local, "wb")) == (FILE *)0) {
			perror(local);
			return 4;
		}
	} else
		out = stdout;

	/* RRQ: opcode, filename NUL, mode NUL */
	put16(req, OP_RRQ);
	reqlen = 2;
	strcpy((char *)req + reqlen, remote);
	reqlen += strlen(remote) + 1;
	strcpy((char *)req + reqlen, mode);
	reqlen += strlen(mode) + 1;

	t0 = now_ms();
	if (send_pkt(s, &server, req, reqlen) < 0)
		return 4;

	for (;;) {
		n = recv_pkt(s, buf, sizeof(buf), &from, timeout_s);
		if (n < 0)
			return 4;
		if (n == 0) {
			if (++retries > max_retries) {
				fprintf(stderr, "tftpget: TIMEOUT waiting for block %u "
				    "after %d retries\n", expect_block, max_retries);
				return 2;
			}
			total_retries++;
			if (!quiet)
				fprintf(stderr, "tftpget: timeout, resending %s\n",
				    have_tid ? "ACK" : "RRQ");
			if (have_tid)
				send_pkt(s, &server, ack, 4);
			else
				send_pkt(s, &server, req, reqlen);
			continue;
		}
		if (from.sin_addr.s_addr != server.sin_addr.s_addr) {
			if (!quiet)
				fprintf(stderr, "tftpget: ignoring packet from %s\n",
				    inet_ntoa(from.sin_addr));
			continue;
		}
		/* The first reply fixes the server's TID (port); later ones must match. */
		if (have_tid && from.sin_port != server.sin_port) {
			unsigned char err[32];
			if (!quiet)
				fprintf(stderr, "tftpget: packet from wrong port %u\n",
				    (unsigned)ntohs(from.sin_port));
			put16(err, OP_ERROR);
			put16(err + 2, 5);
			strcpy((char *)err + 4, "unknown TID");
			send_pkt(s, &from, err, 4 + 12);
			continue;
		}
		if (n < 4) {
			fprintf(stderr, "tftpget: PROTOCOL short packet (%d bytes)\n", n);
			return 3;
		}
		switch (get16(buf)) {
		case OP_ERROR:
			buf[n < (int)sizeof(buf) ? n : (int)sizeof(buf) - 1] = 0;
			fprintf(stderr, "tftpget: ERROR %u %s\n", get16(buf + 2),
			    n > 4 ? (char *)buf + 4 : "");
			return expect_error ? 0 : 1;
		case OP_DATA:
			break;
		default:
			fprintf(stderr, "tftpget: PROTOCOL unexpected opcode %u\n", get16(buf));
			return 3;
		}
		if (!have_tid) {
			server.sin_port = from.sin_port;
			have_tid = 1;
		}
		if (n - 4 > SEGSIZE) {
			fprintf(stderr, "tftpget: PROTOCOL oversized DATA (%d bytes)\n", n - 4);
			return 3;
		}
		if (get16(buf + 2) != (expect_block & 0xffff)) {
			/* A retransmit of the previous block: re-ACK it, don't store it. */
			if (get16(buf + 2) == ((expect_block - 1) & 0xffff)) {
				send_pkt(s, &server, ack, 4);
				continue;
			}
			fprintf(stderr, "tftpget: PROTOCOL got block %u, expected %u\n",
			    get16(buf + 2), expect_block & 0xffff);
			return 3;
		}
		put_data(out, buf + 4, n - 4, netascii);
		blocks++;
		retries = 0;
		put16(ack, OP_ACK);
		put16(ack + 2, expect_block & 0xffff);
		if (send_pkt(s, &server, ack, 4) < 0)
			return 4;
		expect_block++;
		if (n - 4 < SEGSIZE)
			break;
	}
	if (pending_cr)
		put_byte(out, '\r');
	if (fflush(out) != 0 || (out != stdout && fclose(out) != 0)) {
		perror("tftpget: write");
		return 4;
	}
	if (expect_error) {
		fprintf(stderr, "tftpget: expected an ERROR, got the file\n");
		return 3;
	}
	fprintf(stderr, "tftpget: OK %ld bytes %ld blocks %d retries %ld ms sum %08lx\n",
	    total_bytes, blocks, total_retries, now_ms() - t0,
	    (sum_b << 16) | sum_a);
	return 0;
}

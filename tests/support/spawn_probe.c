/* A Rust probe would install SIG_IGN for SIGPIPE before main. */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void report(const char *text) {
    fputs(text, stdout);
    fputc('\n', stdout);
    fflush(stdout);
}

static int descriptor_argument(const char *text) {
    char *end = NULL;
    long value = strtol(text, &end, 10);
    if (end == text || *end != '\0' || value < 0 || value > INT_MAX) {
        return -1;
    }
    return (int)value;
}

int main(int argc, char **argv) {
    if (argc == 3 && strcmp(argv[1], "fd") == 0) {
        int fd = descriptor_argument(argv[2]);
        int flags;
        if (fd < 0) {
            return 2;
        }
        flags = fcntl(fd, F_GETFD);
        if (flags >= 0) {
            printf("FD_OPEN %d\n", flags);
            fflush(stdout);
            return 0;
        }
        if (errno == EBADF) {
            report("FD_CLOSED");
            return 0;
        }
        report("FD_ERR");
        return 0;
    }
    if (argc == 2 && strcmp(argv[1], "sigpipe") == 0) {
        struct sigaction current;
        memset(&current, 0, sizeof(current));
        if (sigaction(SIGPIPE, NULL, &current) != 0) {
            report("SIGPIPE_ERR");
            return 0;
        }
        if (current.sa_handler == SIG_DFL) {
            report("SIGPIPE_DFL");
        } else if (current.sa_handler == SIG_IGN) {
            report("SIGPIPE_IGN");
        } else {
            report("SIGPIPE_OTHER");
        }
        return 0;
    }
    return 2;
}

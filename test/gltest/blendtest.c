/*
 * blendtest: every OpenGL blend configuration IRIX offers, checked against
 * the GL formulas. One frame of 8x8 cells, one configuration a cell: the
 * destination colour is drawn unblended, then the source rectangle under
 * the cell's blend state; the window is read back once at the end.
 *
 * Covers GL 1.1 blending (every legal factor pair), EXT_blend_color (the
 * four constant factors), EXT_blend_subtract, EXT_blend_minmax and
 * EXT_blend_logic_op, plus the colour logic op (GL_COLOR_LOGIC_OP). Groups
 * whose extension is missing are skipped.
 *
 * Board-neutral: written for IMPACT, meant to run unchanged on GR2 (XZ /
 * Extreme) and anything else with an RGBA visual.
 *
 * Output: one line per group (PASS / FAIL / SKIP) and up to 8 failing
 * cells per group; exit status = failing cells (capped at 255).
 *   blendtest [-v] [--hold MS] [--only GROUP]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <X11/Xlib.h>
#include <GL/gl.h>
#include <GL/glx.h>

#define WIN 256
#define CELL 8
#define PER_ROW (WIN / CELL)

#ifndef GL_CONSTANT_COLOR_EXT
#define GL_CONSTANT_COLOR_EXT 0x8001
#define GL_ONE_MINUS_CONSTANT_COLOR_EXT 0x8002
#define GL_CONSTANT_ALPHA_EXT 0x8003
#define GL_ONE_MINUS_CONSTANT_ALPHA_EXT 0x8004
#endif
#ifndef GL_FUNC_ADD_EXT
#define GL_FUNC_ADD_EXT 0x8006
#define GL_MIN_EXT 0x8007
#define GL_MAX_EXT 0x8008
#endif
#ifndef GL_FUNC_SUBTRACT_EXT
#define GL_FUNC_SUBTRACT_EXT 0x800A
#define GL_FUNC_REVERSE_SUBTRACT_EXT 0x800B
#endif
#ifndef GL_LOGIC_OP
#define GL_LOGIC_OP 0x0BF1
#endif

typedef struct {
    int group;            /* index into groups[] */
    GLenum eq;            /* blend equation; 0 = GL_COLOR_LOGIC_OP instead */
    GLenum sf, df;        /* blend factors */
    GLenum op;            /* logic op (logic groups) */
    unsigned char s[4], d[4];
} Cell;

static const char *groups[] = { "add", "subtract", "reverse-subtract", "min", "max",
                                "blend-logic-op", "color-logic-op" };
#define NGROUPS 7

static Display *dpy;
static Window win;
static int have_alpha, ext_color, ext_sub, ext_minmax, ext_logic, verbose;
static Cell cells[PER_ROW * PER_ROW];
static int ncells;

static const GLenum src_factors[] = {
    GL_ZERO, GL_ONE, GL_DST_COLOR, GL_ONE_MINUS_DST_COLOR, GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA, GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA, GL_SRC_ALPHA_SATURATE,
    GL_CONSTANT_COLOR_EXT, GL_ONE_MINUS_CONSTANT_COLOR_EXT, GL_CONSTANT_ALPHA_EXT,
    GL_ONE_MINUS_CONSTANT_ALPHA_EXT,
};
static const GLenum dst_factors[] = {
    GL_ZERO, GL_ONE, GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR, GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA, GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR_EXT, GL_ONE_MINUS_CONSTANT_COLOR_EXT, GL_CONSTANT_ALPHA_EXT,
    GL_ONE_MINUS_CONSTANT_ALPHA_EXT,
};
#define NSF (sizeof src_factors / sizeof src_factors[0])
#define NDF (sizeof dst_factors / sizeof dst_factors[0])

/* The constant colour, exact in bytes. */
static const unsigned char kc8[4] = { 64, 160, 224, 96 };

static const char *fname(GLenum f)
{
    switch (f) {
    case GL_ZERO: return "ZERO";
    case GL_ONE: return "ONE";
    case GL_SRC_COLOR: return "SRC_COLOR";
    case GL_ONE_MINUS_SRC_COLOR: return "1-SRC_COLOR";
    case GL_DST_COLOR: return "DST_COLOR";
    case GL_ONE_MINUS_DST_COLOR: return "1-DST_COLOR";
    case GL_SRC_ALPHA: return "SRC_ALPHA";
    case GL_ONE_MINUS_SRC_ALPHA: return "1-SRC_ALPHA";
    case GL_DST_ALPHA: return "DST_ALPHA";
    case GL_ONE_MINUS_DST_ALPHA: return "1-DST_ALPHA";
    case GL_SRC_ALPHA_SATURATE: return "SAT";
    case GL_CONSTANT_COLOR_EXT: return "CONST_COLOR";
    case GL_ONE_MINUS_CONSTANT_COLOR_EXT: return "1-CONST_COLOR";
    case GL_CONSTANT_ALPHA_EXT: return "CONST_ALPHA";
    case GL_ONE_MINUS_CONSTANT_ALPHA_EXT: return "1-CONST_ALPHA";
    }
    return "?";
}

static int is_const(GLenum f)
{
    return f >= GL_CONSTANT_COLOR_EXT && f <= GL_ONE_MINUS_CONSTANT_ALPHA_EXT;
}

/* Factor for component k (0..3), all values in [0, 1]. */
static double factor(GLenum f, int k, const double *s, const double *d, const double *c)
{
    switch (f) {
    case GL_ZERO: return 0;
    case GL_ONE: return 1;
    case GL_SRC_COLOR: return s[k];
    case GL_ONE_MINUS_SRC_COLOR: return 1 - s[k];
    case GL_DST_COLOR: return d[k];
    case GL_ONE_MINUS_DST_COLOR: return 1 - d[k];
    case GL_SRC_ALPHA: return s[3];
    case GL_ONE_MINUS_SRC_ALPHA: return 1 - s[3];
    case GL_DST_ALPHA: return d[3];
    case GL_ONE_MINUS_DST_ALPHA: return 1 - d[3];
    case GL_SRC_ALPHA_SATURATE: return k == 3 ? 1 : (s[3] < 1 - d[3] ? s[3] : 1 - d[3]);
    case GL_CONSTANT_COLOR_EXT: return c[k];
    case GL_ONE_MINUS_CONSTANT_COLOR_EXT: return 1 - c[k];
    case GL_CONSTANT_ALPHA_EXT: return c[3];
    case GL_ONE_MINUS_CONSTANT_ALPHA_EXT: return 1 - c[3];
    }
    return 1;
}

static unsigned logic(GLenum op, unsigned s, unsigned d)
{
    unsigned v;
    switch (op) {
    case GL_CLEAR: v = 0; break;
    case GL_AND: v = s & d; break;
    case GL_AND_REVERSE: v = s & ~d; break;
    case GL_COPY: v = s; break;
    case GL_AND_INVERTED: v = ~s & d; break;
    case GL_NOOP: v = d; break;
    case GL_XOR: v = s ^ d; break;
    case GL_OR: v = s | d; break;
    case GL_NOR: v = ~(s | d); break;
    case GL_EQUIV: v = ~(s ^ d); break;
    case GL_INVERT: v = ~d; break;
    case GL_OR_REVERSE: v = s | ~d; break;
    case GL_COPY_INVERTED: v = ~s; break;
    case GL_OR_INVERTED: v = ~s | d; break;
    case GL_NAND: v = ~(s & d); break;
    default: v = 0xFF; break; /* GL_SET */
    }
    return v & 0xFF;
}

/* Expected bytes of a cell. */
static void expect(const Cell *c, unsigned char *out)
{
    double s[4], d[4], k[4], v;
    int i;
    for (i = 0; i < 4; i++) {
        s[i] = c->s[i] / 255.0;
        d[i] = c->d[i] / 255.0;
        k[i] = kc8[i] / 255.0;
    }
    for (i = 0; i < 4; i++) {
        if (c->eq == 0 || c->eq == GL_LOGIC_OP) {
            out[i] = (unsigned char)logic(c->op, c->s[i], c->d[i]);
            continue;
        }
        switch (c->eq) {
        case GL_MIN_EXT: v = s[i] < d[i] ? s[i] : d[i]; break;
        case GL_MAX_EXT: v = s[i] > d[i] ? s[i] : d[i]; break;
        case GL_FUNC_SUBTRACT_EXT: v = s[i] * factor(c->sf, i, s, d, k) - d[i] * factor(c->df, i, s, d, k); break;
        case GL_FUNC_REVERSE_SUBTRACT_EXT: v = d[i] * factor(c->df, i, s, d, k) - s[i] * factor(c->sf, i, s, d, k); break;
        default: v = s[i] * factor(c->sf, i, s, d, k) + d[i] * factor(c->df, i, s, d, k); break;
        }
        if (v < 0) v = 0;
        if (v > 1) v = 1;
        out[i] = (unsigned char)(v * 255.0 + 0.5);
    }
}

/* A byte colour that varies with n and component k, never 0 or 255 so
   ZERO / ONE factors and clamps show. */
static unsigned char pat(int n, int k, int salt)
{
    return (unsigned char)(24 + (n * 37 + k * 71 + salt * 113 + n * k * 13) % 208);
}

static void add(int group, GLenum eq, GLenum sf, GLenum df, GLenum op)
{
    Cell *c;
    int k;
    if (ncells >= PER_ROW * PER_ROW)
        return;
    c = &cells[ncells];
    c->group = group;
    c->eq = eq;
    c->sf = sf;
    c->df = df;
    c->op = op;
    for (k = 0; k < 4; k++) {
        c->s[k] = pat(ncells, k, 1);
        c->d[k] = pat(ncells, k, 2);
    }
    ncells++;
}

static int group_on(int g, const char *only)
{
    if (only && !strstr(groups[g], only))
        return 0;
    switch (g) {
    case 1: case 2: return ext_sub;
    case 3: case 4: return ext_minmax;
    case 5: return ext_logic;
    }
    return 1;
}

static void build(const char *only)
{
    static const GLenum eqs[3] = { GL_FUNC_ADD_EXT, GL_FUNC_SUBTRACT_EXT, GL_FUNC_REVERSE_SUBTRACT_EXT };
    unsigned e, i, j;
    GLenum op;
    for (e = 0; e < 3; e++) {
        if (!group_on(e, only))
            continue;
        for (i = 0; i < NSF; i++)
            for (j = 0; j < NDF; j++) {
                if (!ext_color && (is_const(src_factors[i]) || is_const(dst_factors[j])))
                    continue;
                add(e, eqs[e], src_factors[i], dst_factors[j], 0);
            }
    }
    for (i = 0; i < 4; i++) {
        if (group_on(3, only))
            add(3, GL_MIN_EXT, GL_ONE, GL_ONE, 0);
        if (group_on(4, only))
            add(4, GL_MAX_EXT, GL_ONE, GL_ONE, 0);
    }
    for (op = GL_CLEAR; op <= GL_SET; op++) {
        if (group_on(5, only))
            add(5, GL_LOGIC_OP, GL_ONE, GL_ZERO, op);
        if (group_on(6, only))
            add(6, 0, GL_ONE, GL_ZERO, op);
    }
}

static void rect(int n)
{
    int x = (n % PER_ROW) * CELL, y = (n / PER_ROW) * CELL;
    glRecti(x, y, x + CELL, y + CELL);
}

static void draw(void)
{
    int n;
    glDisable(GL_DITHER);
    glClearColor(0, 0, 0, 0);
    glClear(GL_COLOR_BUFFER_BIT);
    /* Destinations, unblended. */
    for (n = 0; n < ncells; n++) {
        glColor4ub(cells[n].d[0], cells[n].d[1], cells[n].d[2], cells[n].d[3]);
        rect(n);
    }
    if (ext_color)
        glBlendColorEXT(kc8[0] / 255.0f, kc8[1] / 255.0f, kc8[2] / 255.0f, kc8[3] / 255.0f);
    for (n = 0; n < ncells; n++) {
        const Cell *c = &cells[n];
        if (c->eq == 0) {
            glEnable(GL_COLOR_LOGIC_OP);
            glLogicOp(c->op);
        } else {
            if (ext_sub || ext_minmax || ext_logic)
                glBlendEquationEXT(c->eq);
            if (c->eq == GL_LOGIC_OP)
                glLogicOp(c->op);
            glBlendFunc(c->sf, c->df);
            glEnable(GL_BLEND);
        }
        glColor4ub(c->s[0], c->s[1], c->s[2], c->s[3]);
        rect(n);
        glDisable(GL_BLEND);
        glDisable(GL_COLOR_LOGIC_OP);
        glLogicOp(GL_COPY);
        if (ext_sub || ext_minmax || ext_logic)
            glBlendEquationEXT(GL_FUNC_ADD_EXT);
    }
    glFinish();
}

static int check(void)
{
    unsigned char *fb = malloc(WIN * WIN * 4);
    int bad[NGROUPS], shown[NGROUPS], total[NGROUPS], n, g, k, fails = 0;
    memset(bad, 0, sizeof bad);
    memset(shown, 0, sizeof shown);
    memset(total, 0, sizeof total);
    glPixelStorei(GL_PACK_ALIGNMENT, 1);
    glReadBuffer(GL_BACK);
    glReadPixels(0, 0, WIN, WIN, GL_RGBA, GL_UNSIGNED_BYTE, fb);
    for (n = 0; n < ncells; n++) {
        const Cell *c = &cells[n];
        int x = (n % PER_ROW) * CELL + CELL / 2, y = (n / PER_ROW) * CELL + CELL / 2;
        const unsigned char *p = fb + (y * WIN + x) * 4;
        unsigned char e[4];
        int ok = 1, ncomp = have_alpha ? 4 : 3;
        /* Destination alpha factors need alpha planes. */
        if (!have_alpha && (c->sf == GL_DST_ALPHA || c->sf == GL_ONE_MINUS_DST_ALPHA ||
                            c->df == GL_DST_ALPHA || c->df == GL_ONE_MINUS_DST_ALPHA ||
                            c->sf == GL_SRC_ALPHA_SATURATE))
            continue;
        expect(c, e);
        total[c->group]++;
        for (k = 0; k < ncomp; k++)
            if (abs(p[k] - e[k]) > (c->eq == 0 || c->eq == GL_LOGIC_OP ? 0 : 3))
                ok = 0;
        if (!ok) {
            bad[c->group]++;
            fails++;
            if (shown[c->group]++ < 8 || verbose)
                printf("  %s: cell %d %s/%s op 0x%x: s %02x%02x%02x%02x d %02x%02x%02x%02x got %02x%02x%02x%02x want %02x%02x%02x%02x\n",
                       groups[c->group], n, fname(c->sf), fname(c->df), c->op,
                       c->s[0], c->s[1], c->s[2], c->s[3], c->d[0], c->d[1], c->d[2], c->d[3],
                       p[0], p[1], p[2], p[3], e[0], e[1], e[2], e[3]);
        }
    }
    for (g = 0; g < NGROUPS; g++) {
        if (!total[g])
            printf("SKIP %s\n", groups[g]);
        else if (bad[g])
            printf("FAIL %s: %d of %d cells\n", groups[g], bad[g], total[g]);
        else
            printf("PASS %s: %d cells\n", groups[g], total[g]);
    }
    free(fb);
    return fails;
}

static Bool wait_map(Display *d, XEvent *e, char *arg)
{
    return e->type == MapNotify && e->xmap.window == (Window)arg;
}

int main(int argc, char **argv)
{
    int attr[] = { GLX_RGBA, GLX_DOUBLEBUFFER, GLX_RED_SIZE, 8, GLX_GREEN_SIZE, 8,
                   GLX_BLUE_SIZE, 8, GLX_ALPHA_SIZE, 8, None };
    int attr_na[] = { GLX_RGBA, GLX_DOUBLEBUFFER, GLX_RED_SIZE, 1, None };
    XVisualInfo *vi;
    XSetWindowAttributes swa;
    XEvent ev;
    GLXContext ctx;
    const char *ext, *only = 0;
    int i, hold = 0, a, fails;
    for (i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "-v"))
            verbose = 1;
        else if (!strcmp(argv[i], "--hold") && i + 1 < argc)
            hold = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--only") && i + 1 < argc)
            only = argv[++i];
        else {
            fprintf(stderr, "usage: blendtest [-v] [--hold MS] [--only GROUP]\n");
            return 255;
        }
    }
    dpy = XOpenDisplay(0);
    if (!dpy)
        return 255;
    vi = glXChooseVisual(dpy, DefaultScreen(dpy), attr);
    if (!vi)
        vi = glXChooseVisual(dpy, DefaultScreen(dpy), attr_na);
    if (!vi)
        return 255;
    glXGetConfig(dpy, vi, GLX_ALPHA_SIZE, &a);
    have_alpha = a >= 8;
    swa.colormap = XCreateColormap(dpy, RootWindow(dpy, vi->screen), vi->visual, AllocNone);
    swa.border_pixel = 0;
    swa.event_mask = StructureNotifyMask;
    win = XCreateWindow(dpy, RootWindow(dpy, vi->screen), 0, 0, WIN, WIN, 0, vi->depth, InputOutput,
                        vi->visual, CWBorderPixel | CWColormap | CWEventMask, &swa);
    XStoreName(dpy, win, "blendtest");
    XMapWindow(dpy, win);
    XIfEvent(dpy, &ev, wait_map, (char *)win);
    ctx = glXCreateContext(dpy, vi, 0, True);
    glXMakeCurrent(dpy, win, ctx);
    ext = (const char *)glGetString(GL_EXTENSIONS);
    ext_color = ext && strstr(ext, "GL_EXT_blend_color") != 0;
    ext_sub = ext && strstr(ext, "GL_EXT_blend_subtract") != 0;
    ext_minmax = ext && strstr(ext, "GL_EXT_blend_minmax") != 0;
    ext_logic = ext && strstr(ext, "GL_EXT_blend_logic_op") != 0;
    printf("visual 0x%x alpha %d; %s; blend_color %d subtract %d minmax %d logic_op %d\n",
           (unsigned)vi->visualid, a, glGetString(GL_RENDERER), ext_color, ext_sub, ext_minmax, ext_logic);
    glViewport(0, 0, WIN, WIN);
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glOrtho(0, WIN, 0, WIN, -1, 1);
    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
    glDrawBuffer(GL_BACK);
    build(only);
    draw();
    fails = check();
    printf("blendtest: %d cells, %d failed\n", ncells, fails);
    if (hold > 0) {
        glXSwapBuffers(dpy, win);
        glFinish();
        usleep(hold * 1000);
    }
    glXMakeCurrent(dpy, None, 0);
    glXDestroyContext(dpy, ctx);
    XCloseDisplay(dpy);
    return fails > 255 ? 255 : fails;
}

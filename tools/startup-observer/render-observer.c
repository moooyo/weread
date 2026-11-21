#define _GNU_SOURCE
#include "render-scene.h"
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <linux/fb.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#ifdef PUBLIC_FRAME_REPLAY
int main(void) {
    static uint8_t input[R_WIDTH * R_HEIGHT];
    if (fread(input,1,sizeof input,stdin) != sizeof input || fgetc(stdin) != EOF || ferror(stdin)) return 2;
    struct observed_scene current = scene(input,R_WIDTH);
    puts(scene_name(current.kind));
    return 0;
}
#else
#define PROD_APP "/mnt/us/weread/app/weread"
#define LOCK "/tmp/weread-launcher.lock"
#define WAIT_IOCTL 0xc008462fUL
#define SECOND UINT64_C(1000000000)
struct geometry { unsigned rotate, gray, xvirtual, yvirtual, page, bytes; };
struct gui { int proc; unsigned pid; uint64_t birth; struct stat elf; };
struct waiting { unsigned long descriptor, argument; };

static int fail(const char *reason) { fprintf(stderr,"render_refused %s\n",reason); return 1; }
static uint64_t now(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC,&value) != 0) _exit(2);
    return (uint64_t)value.tv_sec * SECOND + (uint64_t)value.tv_nsec;
}
static void pause_ms(unsigned number) {
    struct timespec interval = {0,(long)number * 1000000};
    if (nanosleep(&interval,NULL) != 0) _exit(2);
}
static ssize_t text_at(int directory,const char *name,char *buffer,size_t size) {
    int file = openat(directory,name,O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC);
    if (file < 0) return -1;
    struct stat status;
    ssize_t count = -1;
    if (!fstat(file,&status) && S_ISREG(status.st_mode) && status.st_size >= 0 && (uint64_t)status.st_size < size) {
        count = read(file,buffer,size);
        if (count >= 0 && (size_t)count < size) buffer[count] = 0;
        else count = -1;
    }
    close(file);
    return count;
}
static int tag(int stage,const char *leaf,const char *label) {
    char content[20];
    return text_at(stage,leaf,content,sizeof content) == 17 && !memcmp(content,label,16) && content[16] == '\n';
}
static int numeric(const char *start,const char *end,uint64_t *number) {
    if (end <= start || (size_t)(end-start) > 20) return 0;
    uint64_t value = 0;
    for (const char *cursor = start; cursor < end; ++cursor) {
        if (*cursor < '0' || *cursor > '9') return 0;
        unsigned digit = (unsigned)(*cursor-'0');
        if (value > (UINT64_MAX-digit)/10) return 0;
        value = value*10+digit;
    }
    *number = value;
    return value != 0;
}
static uint64_t integer_record(int directory,const char *leaf) {
    char buffer[32]; uint64_t result;
    ssize_t count = text_at(directory,leaf,buffer,sizeof buffer);
    return count > 1 && buffer[count-1] == '\n' && numeric(buffer,buffer+count-1,&result) ? result : 0;
}
static uint64_t proc_birth(int proc) {
    char buffer[4096]; uint64_t result;
    if (text_at(proc,"stat",buffer,sizeof buffer) <= 0) return 0;
    char *cursor = strrchr(buffer,')');
    if (!cursor || cursor[1] != ' ') return 0;
    cursor += 2;
    for (unsigned field=3;field<22;++field) { cursor = strchr(cursor,' '); if (!cursor) return 0; ++cursor; }
    char *end = strchr(cursor,' ');
    return end && numeric(cursor,end,&result) ? result : 0;
}
static int owner(struct gui *gui,int lock) {
    static const char args[] = PROD_APP "\0--kindle\0--device\0scribe\0--data-dir\0/mnt/us/weread/data";
    char observed[sizeof args+1]; struct stat active;
    return integer_record(lock,"application-pid") == gui->pid && integer_record(lock,"application-birth") == gui->birth &&
        proc_birth(gui->proc) == gui->birth && fstatat(gui->proc,"exe",&active,0) == 0 &&
        active.st_dev == gui->elf.st_dev && active.st_ino == gui->elf.st_ino &&
        text_at(gui->proc,"cmdline",observed,sizeof observed) == (ssize_t)sizeof args && !memcmp(observed,args,sizeof args);
}
static int claimed(int stage,int lock,const struct gui *gui) {
    char receipt[320],generation[64],ownership[128],actual[128],nonce[33],tail;
    unsigned long long entry,entry_birth,master,master_birth,runner,runner_birth,pid,birth;
    ssize_t count = text_at(stage,"session",receipt,sizeof receipt);
    if (count < 0) return 0;
    if (count < 2 || receipt[count-1] != '\n' || memchr(receipt,0,(size_t)count)) return -1;
    receipt[count-1] = 0;
    if (sscanf(receipt,"%llu %llu %llu %llu %32[0-9a-f] %llu %llu %llu %llu%c",&entry,&entry_birth,&master,&master_birth,
               nonce,&runner,&runner_birth,&pid,&birth,&tail) != 9 || !entry || !entry_birth || !master || !master_birth ||
        !runner || !runner_birth || strlen(nonce) != 32 || pid != gui->pid || birth != gui->birth) return -1;
    int glen=snprintf(generation,sizeof generation,"1:%s\n",nonce);
    int olen=snprintf(ownership,sizeof ownership,"1:%s:%u:%" PRIu64 "\n",nonce,gui->pid,gui->birth);
    if (text_at(lock,"generation",actual,sizeof actual) != glen || memcmp(actual,generation,(size_t)glen)) return -1;
    return text_at(lock,"application-owned",actual,sizeof actual) == olen && !memcmp(actual,ownership,(size_t)olen) ? 1 : -1;
}
static int bind_gui(struct gui *gui,int lock) {
    uint64_t pid = integer_record(lock,"application-pid"), birth = integer_record(lock,"application-birth");
    if (!pid || pid > INT32_MAX || !birth) return 0;
    char path[40]; snprintf(path,sizeof path,"/proc/%u",(unsigned)pid);
    gui->proc = open(path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if (gui->proc < 0) return 0;
    gui->pid = (unsigned)pid; gui->birth = birth;
    if (owner(gui,lock)) return 1;
    close(gui->proc); gui->proc = -1;
    return 0;
}
static int describe(struct fb_var_screeninfo var,struct fb_fix_screeninfo fix,struct geometry *mode) {
    if (var.xres != R_WIDTH || var.yres != R_HEIGHT || var.xoffset || fix.line_length != R_STRIDE ||
        var.bits_per_pixel != 8 || (var.rotate != 1 && var.rotate != 3) || (var.grayscale != 1 && var.grayscale != 2) ||
        (var.xres_virtual != R_WIDTH && var.xres_virtual != R_STRIDE) ||
        (var.yres_virtual != R_HEIGHT && var.yres_virtual != 2*R_HEIGHT) ||
        (var.yoffset != 0 && var.yoffset != R_HEIGHT) || var.yoffset+R_HEIGHT > var.yres_virtual ||
        fix.smem_len < R_STRIDE*(var.yoffset+R_HEIGHT)) return 0;
    *mode = (struct geometry){var.rotate,var.grayscale,var.xres_virtual,var.yres_virtual,var.yoffset,fix.smem_len};
    return 1;
}
static int fb_mode(int fd,struct geometry *mode) {
    struct fb_var_screeninfo var; struct fb_fix_screeninfo fix;
    return !ioctl(fd,FBIOGET_FSCREENINFO,&fix) && !ioctl(fd,FBIOGET_VSCREENINFO,&var) && describe(var,fix,mode);
}
static int same_mode(struct geometry left,struct geometry right) {
    return left.rotate == right.rotate && left.gray == right.gray && left.page == right.page &&
           left.xvirtual == right.xvirtual && left.yvirtual == right.yvirtual && left.bytes == right.bytes;
}
static int waiting_on_fb(const struct gui *gui,const struct stat *fb,struct waiting *witness) {
    char reading[512],path[48]; struct stat info;
    if (text_at(gui->proc,"syscall",reading,sizeof reading) <= 0) return 0;
    long call; unsigned long descriptor,cmd,argument;
    if (sscanf(reading,"%ld %lx %lx %lx",&call,&descriptor,&cmd,&argument) != 4 || call != SYS_ioctl ||
        cmd != WAIT_IOCTL || descriptor > INT32_MAX || !argument) return 0;
    snprintf(path,sizeof path,"fd/%lu",descriptor);
    if (fstatat(gui->proc,path,&info,0) || !S_ISCHR(info.st_mode) || info.st_rdev != fb->st_rdev ||
        info.st_dev != fb->st_dev || info.st_ino != fb->st_ino) return 0;
    *witness = (struct waiting){descriptor,argument}; return 1;
}
static unsigned long poll_set(const struct gui *gui) {
    char reading[512]; long call; unsigned long array,count;
    if (text_at(gui->proc,"syscall",reading,sizeof reading) <= 0 ||
        sscanf(reading,"%ld %lx %lx",&call,&array,&count) != 3 || !array || count != 3) return 0;
#ifdef SYS_poll
    if (call == SYS_poll) return array;
#endif
#ifdef SYS_ppoll
    if (call == SYS_ppoll) return array;
#endif
#ifdef SYS_ppoll_time64
    if (call == SYS_ppoll_time64) return array;
#endif
    return 0;
}
static int same_scene(struct observed_scene first,struct observed_scene next) {
    return first.kind == next.kind;
}
static int sample(int fb,const volatile uint8_t *map,size_t length,struct geometry *mode,struct observed_scene *result) {
    struct geometry after;
    if (!fb_mode(fb,mode) || (mode->page+R_HEIGHT)*(size_t)R_STRIDE > length) return 0;
    *result = scene(map+mode->page*(size_t)R_STRIDE,R_STRIDE);
    return fb_mode(fb,&after) && same_mode(*mode,after);
}
static int output(struct observed_scene picture,uint64_t negative,uint64_t first,uint64_t confirmed,uint64_t upper) {
    printf("{\"clock\":\"CLOCK_MONOTONIC\",\"scene\":\"%s\",\"evidence\":\"%s\",\"last_negative_ns\":%" PRIu64
           ",\"first_complete_scene_ns\":%" PRIu64 ",\"confirmed_scene_ns\":%" PRIu64 ",\"submit_upper_bound_ns\":%" PRIu64 "}\n",
           scene_name(picture.kind),upper ? "submit_upper_bound" : "memory_only",negative,first,confirmed,upper);
    return fflush(stdout) == 0 ? 0 : 1;
}

int main(int argc,char **argv) {
    if (argc != 2 || strlen(argv[1]) != 16 || strspn(argv[1],"0123456789abcdef") != 16) return fail("label");
    sigset_t alarms;
    if (signal(SIGALRM,SIG_DFL) == SIG_ERR || sigemptyset(&alarms) || sigaddset(&alarms,SIGALRM) ||
        sigprocmask(SIG_UNBLOCK,&alarms,NULL)) return fail("deadline");
    alarm(125);
    const char *label = argv[1]; char path[96];
    snprintf(path,sizeof path,"/mnt/us/.weread-render-%s",label);
    int stage = open(path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if (stage < 0 || !tag(stage,"owner",label) || tag(stage,"tile-dispatch-intent",label) || tag(stage,"render-cancel",label)) return fail("stage");
    struct stat st,fb_info; struct gui gui = {.proc=-1};
    if (!lstat(LOCK,&st) || errno != ENOENT || lstat(PROD_APP,&gui.elf) || !S_ISREG(gui.elf.st_mode)) return fail("not_idle");
    int fb = open("/dev/fb0",O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC); struct geometry mode;
    if (fb < 0 || fstat(fb,&fb_info) || !S_ISCHR(fb_info.st_mode) || !fb_mode(fb,&mode)) return fail("geometry");
    size_t length = mode.bytes < 2*(size_t)R_STRIDE*R_HEIGHT ? mode.bytes : 2*(size_t)R_STRIDE*R_HEIGHT;
    const volatile uint8_t *map = mmap(NULL,length,PROT_READ,MAP_SHARED,fb,0);
    struct observed_scene picture;
    if (map == MAP_FAILED || !sample(fb,map,length,&mode,&picture) || picture.kind != NO_SCENE) return fail("initial_screen");
    puts("ARMED"); if (fflush(stdout)) return fail("output");
    uint64_t deadline = now()+90*SECOND;
    while (!tag(stage,"tile-dispatch-intent",label)) {
        if (now() >= deadline || tag(stage,"render-cancel",label)) return fail("intent");
        pause_ms(25);
    }
    deadline = now()+30*SECOND;
    int lock = -1;
    uint64_t negative = 0, first = 0, saw_wait = 0, upper = 0, end = 0;
    struct observed_scene candidate = {0}; struct geometry candidate_mode = {0};
    while (now() < deadline) {
        if (tag(stage,"render-cancel",label) || !tag(stage,"owner",label)) return fail("cancelled");
        if (lock < 0) lock = open(LOCK,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        if (gui.proc < 0 && (lock < 0 || !bind_gui(&gui,lock))) { pause_ms(10); continue; }
        if (!owner(&gui,lock)) return fail("app_changed");
        uint64_t before = now(); struct waiting active = {0}, current = {0};
        unsigned long polling = saw_wait ? poll_set(&gui) : 0;
        int waiting = !saw_wait && waiting_on_fb(&gui,&fb_info,&active);
        if (!sample(fb,map,length,&mode,&picture)) return fail("geometry_changed");
        int still_waiting = waiting && waiting_on_fb(&gui,&fb_info,&current) &&
            active.descriptor == current.descriptor && active.argument == current.argument;
        int still_polling = polling && poll_set(&gui) == polling;
        uint64_t after = now();
        if (picture.kind == NO_SCENE) { negative=before; candidate=picture; first=saw_wait=upper=end=0; }
        else if (!negative) return fail("already_matches");
        else {
            if (!first) { candidate=picture; candidate_mode=mode; first=after; }
            else if (!same_scene(candidate,picture) || !same_mode(candidate_mode,mode)) return fail("ambiguous_scene");
            if (still_waiting && !saw_wait) saw_wait=after;
            if (still_polling && before>saw_wait && !upper) upper=after;
            if (after > first+20000000) {
                end=after;
                int attribution = claimed(stage,lock,&gui);
                if (attribution < 0) return fail("wrong_session");
                if (attribution && (upper || end-first >= 1800000000)) {
                    if (!owner(&gui,lock) || !tag(stage,"owner",label) || tag(stage,"render-cancel",label)) return fail("app_changed");
                    return output(candidate,negative,first,end,upper);
                }
            }
        }
        pause_ms(10);
    }
    return fail("no_complete_scene");
}
#endif

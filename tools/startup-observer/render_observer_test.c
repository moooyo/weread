#define main render_observer_entry
#include "render-observer.c"
#undef main
#undef NDEBUG
#include <assert.h>

static void record(int directory,const char *leaf,const char *value) {
    int file=openat(directory,leaf,O_WRONLY|O_CREAT|O_TRUNC|O_NOFOLLOW,0600);
    assert(file >= 0);
    assert(write(file,value,strlen(value)) == (ssize_t)strlen(value));
    assert(close(file) == 0);
}
int main(void) {
    struct fb_var_screeninfo var={.xres=R_WIDTH,.yres=R_HEIGHT,.xres_virtual=R_WIDTH,.yres_virtual=2*R_HEIGHT,
        .bits_per_pixel=8,.grayscale=1,.rotate=1};
    struct fb_fix_screeninfo fix={.line_length=R_STRIDE,.smem_len=2*R_STRIDE*R_HEIGHT};
    struct geometry initial,changed;
    assert(describe(var,fix,&initial));
    var.rotate=3; var.xres_virtual=R_STRIDE; var.yoffset=R_HEIGHT;
    assert(describe(var,fix,&changed) && !same_mode(initial,changed));
    var.rotate=2; assert(!describe(var,fix,&changed));
    var.rotate=1; var.xres_virtual=R_WIDTH; var.yoffset=0; var.grayscale=2;
    assert(describe(var,fix,&changed) && !same_mode(initial,changed));
    var.grayscale=0; assert(!describe(var,fix,&changed));
    var.grayscale=1; var.yoffset=1; assert(!describe(var,fix,&changed));
    var.yoffset=R_HEIGHT; fix.smem_len=R_STRIDE*R_HEIGHT; assert(!describe(var,fix,&changed));

    int directory=open(".",O_DIRECTORY|O_RDONLY); assert(directory>=0);
    const char *label="0123456789abcdef";
    record(directory,"owner","0123456789abcdef\n"); assert(tag(directory,"owner",label));
    assert(symlinkat("owner",directory,"render-cancel")==0);
    assert(!tag(directory,"render-cancel",label)); assert(unlinkat(directory,"render-cancel",0)==0);
    record(directory,"render-cancel","fedcba9876543210\n"); assert(!tag(directory,"render-cancel",label));
    record(directory,"render-cancel","0123456789abcdef\n"); assert(tag(directory,"render-cancel",label));
    struct gui gui={.proc=directory,.pid=321,.birth=789};
    assert(!claimed(directory,directory,&gui));
    record(directory,"generation","1:0123456789abcdef0123456789abcdef\n");
    record(directory,"application-owned","1:0123456789abcdef0123456789abcdef:321:789\n");
    record(directory,"session","6 12 7 28 0123456789abcdef0123456789abcdef 8 52 321 789\n");
    assert(claimed(directory,directory,&gui)==1);
    gui.birth++; assert(claimed(directory,directory,&gui)<0); gui.birth--;
    record(directory,"generation","2:0123456789abcdef0123456789abcdef\n");
    assert(claimed(directory,directory,&gui)<0);
    int own_proc=open("/proc/self",O_RDONLY|O_DIRECTORY); assert(own_proc>=0 && proc_birth(own_proc)>0); close(own_proc);
    assert(mkdirat(directory,"fd",0700)==0 && symlinkat("/dev/null",directory,"fd/7")==0);
    struct stat expected; assert(stat("/dev/null",&expected)==0); struct waiting wait;
    char trace[128]; snprintf(trace,sizeof trace,"%ld 0x7 0x%lx 0x1234 0 0 0 0 0\n",(long)SYS_ioctl,WAIT_IOCTL);
    record(directory,"syscall",trace); assert(waiting_on_fb(&gui,&expected,&wait));
    assert(wait.descriptor==7 && wait.argument==0x1234);
    assert(stat("/dev/zero",&expected)==0); assert(!waiting_on_fb(&gui,&expected,&wait));
    assert(stat("/dev/null",&expected)==0); record(directory,"syscall","running\n"); assert(!waiting_on_fb(&gui,&expected,&wait));
    snprintf(trace,sizeof trace,"%ld 0x7 0x0 0x1234\n",(long)SYS_ioctl);
    record(directory,"syscall",trace); assert(!waiting_on_fb(&gui,&expected,&wait));
#ifdef SYS_ppoll
    snprintf(trace,sizeof trace,"%ld 0xabcd 0x3 0x100 0x0\n",(long)SYS_ppoll);
    record(directory,"syscall",trace); assert(poll_set(&gui)==0xabcd && !waiting_on_fb(&gui,&expected,&wait));
    snprintf(trace,sizeof trace,"%ld 0xabcd 0x2 0x100 0x0\n",(long)SYS_ppoll);
    record(directory,"syscall",trace); assert(!poll_set(&gui));
#endif
    printf("Geometry, stage ownership, generation, process identity and WAIT discrimination passed.\n");
    close(directory); return 0;
}

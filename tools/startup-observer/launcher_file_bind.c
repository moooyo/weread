#define _GNU_SOURCE
#define _FILE_OFFSET_BITS 64
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifdef BIND_TEST_USERSTORE
#define VOLUME BIND_TEST_USERSTORE
#define PRIVATE BIND_TEST_TMP
#else
#define VOLUME "/mnt/us"
#define PRIVATE "/var/tmp"
#endif

struct leaf { struct stat meta; unsigned mount; };
struct intent {
    char boot[37];
    unsigned long long namespace_dev, namespace_ino, root_dev, root_ino;
    unsigned root_mount, base, source_major, source_minor;
    char root[PATH_MAX];
    struct stat source, target;
};
struct table {
    unsigned base, major, minor, top, parent, top_major, top_minor, mounts, aliases;
    int propagates, foreign;
    char source_root[PATH_MAX], top_root[PATH_MAX];
};
struct operation {
    int stock, authority;
    const char *label, *profile;
    char stage[PATH_MAX], private[PATH_MAX], source[PATH_MAX], target[PATH_MAX];
    char home[PATH_MAX], source_relative[PATH_MAX];
};

static const char *reason = "stage";
static int refuse(const char *code) { reason = code; return 0; }
static int kernel_refusal(const char *name,int error) {
    static char diagnostic[48];
    snprintf(diagnostic,sizeof(diagnostic),"%s_%d",name,error>0&&error<4096?error:EIO);
    return refuse(diagnostic);
}
static int proc_failure(int error) { return refuse(error==EPERM||error==EACCES?"native_proc_permission":"native_proc_read"); }
static int build(char *buffer, size_t capacity, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int used = vsnprintf(buffer, capacity, format, args);
    va_end(args);
    return used >= 0 && (size_t)used < capacity;
}
static int digits(const char *text) { return *text && !text[strspn(text, "0123456789")]; }
static int same(const struct stat *first, const struct stat *second) {
    return first->st_dev == second->st_dev && first->st_ino == second->st_ino;
}
static int identity(const struct stat *first, const struct stat *second) {
    return same(first, second) && first->st_mode == second->st_mode && first->st_nlink == second->st_nlink &&
        first->st_uid == second->st_uid && first->st_gid == second->st_gid && first->st_size == second->st_size &&
        first->st_mtim.tv_sec == second->st_mtim.tv_sec && first->st_mtim.tv_nsec == second->st_mtim.tv_nsec;
}
static int mount_id(int fd, unsigned *id) {
    char path[64], line[192], extra;
    if (!build(path, sizeof(path), "/proc/self/fdinfo/%d", fd)) return 0;
    FILE *file = fopen(path, "r");
    if (!file) return 0;
    int seen = 0;
    while (fgets(line, sizeof(line), file)) if (!strncmp(line, "mnt_id:", 7)) {
        if (++seen != 1 || sscanf(line, "mnt_id: %u %c", id, &extra) != 1 || !*id) { seen = 0; break; }
    }
    int result = seen == 1 && !ferror(file);
    fclose(file);
    return result;
}
static int regular(const char *path, struct leaf *current) {
    int fd = open(path, O_PATH | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return 0;
    struct stat name;
    int result = !fstat(fd, &current->meta) && !lstat(path, &name) && identity(&current->meta, &name) &&
        S_ISREG(name.st_mode) && name.st_nlink == 1 && name.st_size > 0 && name.st_size <= 1024*1024 && mount_id(fd, &current->mount);
    close(fd);
    return result;
}
static uint64_t nonreusable_mount(const char *path) {
#ifdef SYS_statx
    struct statx info={0};
    _Static_assert(sizeof(struct statx)==256 && offsetof(struct statx,stx_dev_minor)==140,"unexpected Linux statx layout");
    const unsigned unique=0x4000U;
    if (!syscall(SYS_statx,AT_FDCWD,path,AT_SYMLINK_NOFOLLOW,unique,&info) && (info.stx_mask&unique)) {
        uint64_t id;
        memcpy(&id,(const char *)&info+144,sizeof(id));
        return id;
    }
#else
    (void)path;
#endif
    return 0;
}
static int plain_dir(const char *path, int private) {
    char actual[PATH_MAX];
    struct stat info;
    return realpath(path, actual) && !strcmp(actual,path) && !lstat(path,&info) && S_ISDIR(info.st_mode) &&
        (!private || (info.st_uid == 0 && (info.st_mode & 07777) == 0700));
}
static int leaf_absent(const char *path) { struct stat info; return lstat(path, &info) && errno == ENOENT; }
static int at_absent(int dir, const char *name) { struct stat info; return fstatat(dir,name,&info,AT_SYMLINK_NOFOLLOW) && errno==ENOENT; }
static int bounded_read(int fd, char *buffer, size_t capacity) {
    size_t length = 0;
    while (length < capacity) {
        ssize_t count = read(fd, buffer+length, capacity-length);
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) return -1;
        if (!count) { buffer[length]=0; return (int)length; }
        length += (size_t)count;
    }
    return -1;
}
static int text_file(const char *path, char *buffer, size_t capacity) {
    int fd = open(path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) return -1;
    int count = bounded_read(fd, buffer, capacity);
    close(fd);
    return count;
}
static int equals_text(const char *path, const char *text) {
    char buffer[256];
    struct stat named;
    return !lstat(path, &named) && S_ISREG(named.st_mode) && named.st_nlink == 1 &&
        named.st_size == (off_t)strlen(text) && text_file(path,buffer,sizeof(buffer)-1) == (int)strlen(text) && !strcmp(buffer,text);
}
static int equals_file(const char *left, const char *right) {
    struct leaf first, second;
    if (!regular(left,&first) || !regular(right,&second) || first.meta.st_size != second.meta.st_size) return 0;
    int one=open(left,O_RDONLY|O_NOFOLLOW|O_CLOEXEC), two=open(right,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    struct stat current;
    int valid=one>=0 && two>=0 && !fstat(one,&current) && identity(&current,&first.meta) && !fstat(two,&current) && identity(&current,&second.meta);
    for (off_t offset=0; valid && offset<first.meta.st_size;) {
        char a[4096], b[4096];
        size_t count=(size_t)(first.meta.st_size-offset); if (count>sizeof(a)) count=sizeof(a);
        valid=pread(one,a,count,offset)==(ssize_t)count && pread(two,b,count,offset)==(ssize_t)count && !memcmp(a,b,count);
        offset+=(off_t)count;
    }
    if (one>=0) close(one);
    if (two>=0) close(two);
    return valid;
}
static int owns(const char *base, const char *label, int private) {
    char name[PATH_MAX], content[24]; struct stat info;
    return plain_dir(base,private) && build(name,sizeof(name),"%s/owner",base) && build(content,sizeof(content),"%s\n",label) &&
        equals_text(name,content) && !lstat(name,&info) && (!private || (info.st_uid==0 && (info.st_mode&07777)==0600));
}
static int paths(struct operation *op) {
    return build(op->stage,sizeof(op->stage),"%s/.weread-%s-%s",VOLUME,op->stock?"render":"bind-diagnostic",op->label) &&
        (op->stock ? build(op->private,sizeof(op->private),"%s/weread-stock-selector-bind",PRIVATE) :
                     build(op->private,sizeof(op->private),"%s/weread-bind-diagnostic-%s",PRIVATE,op->label)) &&
        build(op->home,sizeof(op->home),"%s/weread",VOLUME) &&
        build(op->source,sizeof(op->source),"%s/%s",op->stage,op->stock?"selector-original-slot":"source") &&
        build(op->target,sizeof(op->target),"%s/%s",op->stock?op->home:op->stage,op->stock?"launch.sh":"target") &&
        build(op->source_relative,sizeof(op->source_relative),"%s",op->source+strlen(VOLUME));
}
static int nested(const char *path, const char *parent) {
    size_t length=strlen(parent);
    return !strncmp(path,parent,length) && path[length]=='/';
}
static int positive(const char *word, unsigned *value) {
    if (!digits(word) || strlen(word)>10) return 0;
    char *end; errno=0; unsigned long parsed=strtoul(word,&end,10);
    if (errno || *end || !parsed || parsed>UINT_MAX) return 0;
    *value=(unsigned)parsed; return 1;
}
static int propagation(char *cursor,int *unsafe) {
    char *flag;
    while ((flag=strtok_r(NULL," ",&cursor))) {
        if (!strcmp(flag,"-")) return 1;
        if (!strncmp(flag,"shared:",7)||!strncmp(flag,"master:",7)||!strncmp(flag,"propagate_from:",15)||!strcmp(flag,"unbindable")) *unsafe=1;
    }
    return 0;
}
static int mounts(const struct operation *op, struct table *result) {
    memset(result,0,sizeof(*result));
    FILE *file=fopen("/proc/self/mountinfo","r"); if (!file) return refuse("mount_table");
    char *line=NULL; size_t room=0; int good=1; unsigned pass=0;
    for (pass=0; pass<2 && good; pass++) {
        if (pass) { if (fseek(file,0,SEEK_SET)) {good=0;break;} }
        unsigned lines=0;
        while (getline(&line,&room,file)>=0) {
            if (++lines>8192 || strlen(line)>32768) {good=0;break;}
            char *cursor=NULL,*id=strtok_r(line," ",&cursor),*parent=strtok_r(NULL," ",&cursor),*dev=strtok_r(NULL," ",&cursor);
            char *root=strtok_r(NULL," ",&cursor), *point=strtok_r(NULL," ",&cursor), *opts=strtok_r(NULL," ",&cursor),extra;
            unsigned cur=0,up=0,major=0,minor=0;
            if (!id||!parent||!dev||!root||!point||!opts||!positive(id,&cur)||!positive(parent,&up)||
                sscanf(dev,"%u:%u%c",&major,&minor,&extra)!=2||root[0]!='/'||point[0]!='/') {good=0;break;}
            if (!pass && !strcmp(point,VOLUME)) {
                if (result->base || strchr(root,'\\') || !build(result->source_root,sizeof(result->source_root),"%s%s",!strcmp(root,"/")?"":root,op->source_relative)) {good=0;break;}
                result->base=cur;result->major=major;result->minor=minor;
                if (!propagation(cursor,&result->propagates)) {good=0;break;}
            } else if (pass) {
                if (!strcmp(point,op->target)) {
                    ++result->mounts;result->top=cur;result->parent=up;result->top_major=major;result->top_minor=minor;
                    if (!build(result->top_root,sizeof(result->top_root),"%s",root)) {good=0;break;}
                    if (!propagation(cursor,&result->propagates)) {good=0;break;}
                } else if (!strcmp(point,op->stage) || nested(point,op->stage) || (op->stock && (!strcmp(point,op->home)||nested(point,op->home)))) result->foreign=1;
                if (major==result->major && minor==result->minor && !strcmp(root,result->source_root) && strcmp(point,op->target)) ++result->aliases;
                if (result->base==cur && (strcmp(point,VOLUME) || major!=result->major || minor!=result->minor)) {good=0;break;}
            }
        }
        if (ferror(file) || !result->base) good=0;
    }
    free(line);fclose(file);
    if (!good) return refuse("mount_table");
    if (result->propagates) return refuse("propagation");
    return !(result->foreign||result->aliases||result->mounts>1) || refuse("foreign_mount");
}

static int task_kind(int task) {
    int fd=openat(task,"stat",O_RDONLY|O_CLOEXEC); if (fd<0) return -1;
    char buffer[4096]; int size=bounded_read(fd,buffer,sizeof(buffer)-1);close(fd);
    char *tail=size>0?strrchr(buffer,')'):NULL; char state;unsigned long flags;
    if (!tail || sscanf(tail+1," %c %*d %*d %*d %*d %*d %lu",&state,&flags)!=2) return -1;
    return state=='Z'||state=='X'||(flags&0x00200000UL)?0:1;
}
static int task_view(int task, const struct stat *ns, const struct stat *root, unsigned root_mount) {
    int kind=task_kind(task); if (kind==0) return 1; if (kind<0) return proc_failure(errno);
    int handle=openat(task,"ns/mnt",O_RDONLY|O_CLOEXEC); struct stat found;
    if (handle<0) return proc_failure(errno);
    if (fstat(handle,&found)) { int error=errno;close(handle);return proc_failure(error); }
    int matches=same(ns,&found);close(handle);
    if (!matches) return refuse("namespace");
    handle=openat(task,"root",O_PATH|O_DIRECTORY|O_CLOEXEC);
    if (handle<0) return proc_failure(errno);
    unsigned id=0;
    if (fstat(handle,&found)||!mount_id(handle,&id)) { int error=errno;close(handle);return proc_failure(error); }
    matches=same(root,&found) && id==root_mount;
    close(handle);
    return matches || refuse("native_view");
}
static int scan_tasks(const struct stat *ns, const struct stat *root,unsigned root_mount) {
    DIR *processes=opendir("/proc");if (!processes) return proc_failure(errno);
    unsigned count=0;int init=0,self=0,ok=1;struct dirent *process;
    while ((errno=0,process=readdir(processes))) {
        if (!digits(process->d_name)) continue;
        char path[96];if (!build(path,sizeof(path),"%s/task",process->d_name)) {ok=0;break;}
        int group=openat(dirfd(processes),path,O_DIRECTORY|O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
        if (group<0 && (errno==ENOENT||errno==ESRCH)) continue;
        DIR *threads=group<0?NULL:fdopendir(group); if (!threads) {int error=errno;if(group>=0)close(group);ok=proc_failure(error);break;}
        struct dirent *thread;
        while ((errno=0,thread=readdir(threads))) {
            if (!digits(thread->d_name)) continue;
            if (++count>16384) {ok=refuse("native_proc_limit");break;}
            int task=openat(group,thread->d_name,O_DIRECTORY|O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
            if (task<0 && (errno==ENOENT||errno==ESRCH)) continue;
            if(task<0){ok=proc_failure(errno);break;}
            errno=0; ok=task_view(task,ns,root,root_mount);
            int gone=errno==ENOENT||errno==ESRCH;
            if (!ok && gone && task>=0 && task_kind(task)<=0) ok=1;
            if (task>=0) close(task);
            if (!ok) break;
            if (!strcmp(thread->d_name,"1")) init=1;
            if (strtoul(thread->d_name,NULL,10)==(unsigned)getpid()) self=1;
        }
        if (errno && !thread) ok=proc_failure(errno);
        closedir(threads);
        if (!ok) break;
    }
    if (errno && !process) ok=proc_failure(errno);
    closedir(processes);
    if (!ok) return 0;
    return (init && self) || refuse("native_service");
}
static int views(struct intent *view, int every) {
    struct stat ns,root;unsigned id=0;int fd=open("/",O_PATH|O_DIRECTORY|O_CLOEXEC);
    int ok=fd>=0 && !fstat(fd,&root) && mount_id(fd,&id) && !stat("/proc/self/ns/mnt",&ns);
    if (fd>=0) close(fd);
    if (!ok) return proc_failure(errno);
    view->namespace_dev=ns.st_dev;view->namespace_ino=ns.st_ino;view->root_dev=root.st_dev;view->root_ino=root.st_ino;view->root_mount=id;
    return !every || (scan_tasks(&ns,&root,id) && scan_tasks(&ns,&root,id));
}
static int boot(struct intent *state) {
    char text[52]; if (text_file("/proc/sys/kernel/random/boot_id",text,sizeof(text)-1)!=37||text[36]!='\n') return refuse("native_service");
    text[36]=0;if (strspn(text,"0123456789abcdef-")!=36) return refuse("native_service");
    memcpy(state->boot,text,37);return 1;
}
static int context(const struct intent *one,const struct intent *two) {
    return !strcmp(one->boot,two->boot) && one->namespace_dev==two->namespace_dev && one->namespace_ino==two->namespace_ino &&
        one->root_dev==two->root_dev && one->root_ino==two->root_ino && one->root_mount==two->root_mount &&
        one->base==two->base && one->source_major==two->source_major && one->source_minor==two->source_minor && !strcmp(one->root,two->root);
}

static int authority(struct operation *op,int optional) {
    if (optional && leaf_absent(op->private)) return 1;
    if (!owns(op->private,op->label,1)) return refuse("journal");
    op->authority=open(op->private,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    struct stat open_dir,named;
    if (op->authority<0 || fstat(op->authority,&open_dir) || lstat(op->private,&named) || !same(&named,&open_dir)) return refuse("journal");
    return !flock(op->authority,LOCK_EX|LOCK_NB) || refuse("contended");
}

static int read_record(int dir,const char *name,char *out,size_t capacity) {
    if (dir<0 || at_absent(dir,name)) return 0;
    int fd=openat(dir,name,O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC);struct stat first,named;
    if (fd<0) return -1;
    int valid=!fstat(fd,&first)&&!fstatat(dir,name,&named,AT_SYMLINK_NOFOLLOW)&&identity(&first,&named)&&
        first.st_uid==0 && S_ISREG(first.st_mode) && (first.st_mode&07777)==0600 && first.st_nlink==1 && first.st_size>0 && first.st_size<(off_t)capacity;
    int size=valid?bounded_read(fd,out,capacity-1):-1; close(fd);
    return size>0 && size==first.st_size && strlen(out)==(size_t)size ? size : -1;
}
static int record(int dir,const char *name,const char *text) {
    char next[64];if (!build(next,sizeof(next),"%s.next",name) || !at_absent(dir,name)) return refuse("journal");
    int fd=openat(dir,next,O_WRONLY|O_NOFOLLOW|O_CREAT|O_EXCL|O_CLOEXEC,0600);if(fd<0)return refuse("journal");
    size_t count=strlen(text);int ok=write(fd,text,count)==(ssize_t)count && !fsync(fd);
    int closed=close(fd);
    if(!ok||closed||!at_absent(dir,name)||renameat(dir,next,dir,name)||fsync(dir))return refuse("journal");
    return 1;
}
static int project(const struct stat *meta,char *out,size_t capacity) {
    return build(out,capacity,"%ju:%ju:%ju:%ju:%ju:%ju:%jd:%jd:%ld",(uintmax_t)meta->st_dev,(uintmax_t)meta->st_ino,
        (uintmax_t)meta->st_mode,(uintmax_t)meta->st_uid,(uintmax_t)meta->st_gid,(uintmax_t)meta->st_nlink,
        (intmax_t)meta->st_size,(intmax_t)meta->st_mtim.tv_sec,meta->st_mtim.tv_nsec);
}
static int unproject(const char *input,struct stat *meta) {
    uintmax_t dev,ino,mode,uid,gid,links;intmax_t size,mtime;long nanos;char tail,verify[256];
    if(sscanf(input,"%ju:%ju:%ju:%ju:%ju:%ju:%jd:%jd:%ld%c",&dev,&ino,&mode,&uid,&gid,&links,&size,&mtime,&nanos,&tail)!=9) return 0;
    meta->st_dev=dev;meta->st_ino=ino;meta->st_mode=mode;meta->st_uid=uid;meta->st_gid=gid;meta->st_nlink=links;
    meta->st_size=size;meta->st_mtim.tv_sec=mtime;meta->st_mtim.tv_nsec=nanos;
    return ino && links==1 && S_ISREG((mode_t)mode) && size>0 && size<=1024*1024 && nanos>=0 && nanos<1000000000 &&
        project(meta,verify,sizeof(verify)) && !strcmp(input,verify);
}
static int intent_text(const struct operation *op,const struct intent *state,char *text,size_t capacity) {
    char first[256],second[256];return project(&state->source,first,sizeof(first))&&project(&state->target,second,sizeof(second)) &&
        build(text,capacity,"bind-v1 %s %s %s %llu %llu %llu %llu %u %u %u:%u\n%s\n%s\n%s\n",op->profile,op->label,state->boot,
        state->namespace_dev,state->namespace_ino,state->root_dev,state->root_ino,state->root_mount,state->base,state->source_major,state->source_minor,
        state->root,first,second);
}
static int decode(const struct operation *op,char *text,struct intent *state) {
    char verify[PATH_MAX+1024],original[PATH_MAX+1024]; if(!build(original,sizeof(original),"%s",text)) return 0;
    char *cursor=text,*header=strsep(&cursor,"\n"),*root=strsep(&cursor,"\n"),*from=strsep(&cursor,"\n"),*to=strsep(&cursor,"\n");
    if(!header||!root||!from||!to||!cursor||*cursor) return 0;
    char profile[9],label[17],extra;
    return sscanf(header,"bind-v1 %8s %16s %36s %llu %llu %llu %llu %u %u %u:%u%c",profile,label,state->boot,
        &state->namespace_dev,&state->namespace_ino,&state->root_dev,&state->root_ino,&state->root_mount,&state->base,&state->source_major,&state->source_minor,&extra)==11 &&
        !strcmp(profile,op->profile) && !strcmp(label,op->label) && root[0]=='/' && build(state->root,sizeof(state->root),"%s",root) &&
        unproject(from,&state->source) && unproject(to,&state->target) && !same(&state->source,&state->target) &&
        intent_text(op,state,verify,sizeof(verify)) && !strcmp(verify,original);
}
static int current_views(const struct operation *op,struct intent *now,struct table *table,int full) {
    if(!mounts(op,table)||!views(now,full)||!boot(now)) return 0;
    now->base=table->base;now->source_major=table->major;now->source_minor=table->minor;
    return build(now->root,sizeof(now->root),"%s",table->source_root) || refuse("mount_table");
}
static int public_files(const struct operation *op,const struct leaf *source,const struct leaf *target,int bound,int cheap) {
    if (cheap) return 1;
    if (!op->stock) {
        char expected[32];build(expected,sizeof(expected),"%s:S\n",op->label);
        if(!equals_text(op->source,expected)) return refuse("files");
        build(expected,sizeof(expected),"%s:%c\n",op->label,bound?'S':'T');
        return equals_text(op->target,expected) || refuse("files");
    }
    char path[PATH_MAX],wanted[256];
    if(!build(path,sizeof(path),"%s/selector-stat",op->stage) || !build(wanted,sizeof(wanted),"%ju:%ju:%jd:%o:%u:%u:%jd:%ju\n",
        (uintmax_t)source->meta.st_dev,(uintmax_t)source->meta.st_ino,(intmax_t)source->meta.st_size,(unsigned)(source->meta.st_mode&07777),
        source->meta.st_uid,source->meta.st_gid,(intmax_t)source->meta.st_mtim.tv_sec,(uintmax_t)source->meta.st_nlink)||!equals_text(path,wanted))return refuse("files");
    return bound || ((target->meta.st_mode & 0111) && build(path,sizeof(path),"%s/launch-reference",op->stage) && equals_file(op->target,path)) || refuse("files");
}
static int probe(struct operation *op,const char *command,int *bound) {
    int cheap=!strcmp(command,"current"),clean=!strcmp(command,"restore")||!strcmp(command,"absent"),pre=!strcmp(command,"prepare");
    int activate=!strcmp(command,"bind");
    if (getuid()!=0 || geteuid()!=0 || !plain_dir(VOLUME,0)||!plain_dir(PRIVATE,0) || (op->stock&&!plain_dir(op->home,0))) return refuse("stage");
    if (!authority(op,!activate&&!cheap)) return 0;
    struct intent now={0},saved={0};struct table table={0};
    char text[PATH_MAX+1024],active[64];
    int recorded=read_record(op->authority,"bind-journal",text,sizeof(text));
    int has_active=read_record(op->authority,"bind-active",active,sizeof(active));
    if(recorded<0||has_active<0||(!recorded&&has_active)||(recorded&&!decode(op,text,&saved))) return refuse("journal");
    if (op->authority>=0 && !recorded && !at_absent(op->authority,"bind-active.next")) return refuse("journal");
    int strict=op->stock || !(clean||!strcmp(command,"status")) || recorded;
    if(!current_views(op,&now,&table,!cheap && strict))return 0;
    if(!recorded && table.mounts) return refuse("foreign_mount");
    if(!strcmp(command,"absent") && leaf_absent(op->stage) && op->authority<0 && op->stock) {
        struct leaf production;
        return views(&now,1) && regular(op->target,&production) && production.mount==table.base;
    }
    if (!owns(op->stage,op->label,0)) return refuse("stage");
    struct leaf source,target;
    if(!regular(op->source,&source) || !regular(op->target,&target) || source.mount!=table.base ||
        major(source.meta.st_dev)!=now.source_major || minor(source.meta.st_dev)!=now.source_minor ||
        (!table.mounts && (target.mount!=table.base || same(&source.meta,&target.meta)))) return refuse("files");
    now.source=source.meta;now.target=target.meta;
    if(recorded && (!context(&now,&saved) || !identity(&source.meta,&saved.source)))return refuse("provenance");
    if (table.mounts) {
        if (!recorded || table.parent!=now.base || table.top_major!=now.source_major || table.top_minor!=now.source_minor ||
            strcmp(table.top_root,now.root) || target.mount!=table.top || !identity(&target.meta,&saved.source)) return refuse("foreign_mount");
        if (has_active) {
            char expected[64];build(expected,sizeof(expected),"%s %u %" PRIu64 "\n",op->label,table.top,nonreusable_mount(op->target));
            if (strcmp(active,expected)) return refuse("foreign_mount");
        }
    } else if (recorded && !identity(&target.meta,&saved.target)) return refuse("provenance");
    if(!public_files(op,&source,&target,table.mounts,cheap))return 0;
    *bound=table.mounts!=0;
    if (cheap) return (*bound && has_active) || refuse("unknown_state");
    if (pre || activate) {
        if(*bound||recorded||(op->authority>=0&&!at_absent(op->authority,"bind-journal.next")))return refuse("journal");
        if (pre) return 1;
        if(!intent_text(op,&now,text,sizeof(text))||!record(op->authority,"bind-journal",text))return 0;
        struct intent final={0};struct table prior={0};struct leaf first,second;
        if (!current_views(op,&final,&prior,1)||prior.mounts||!context(&now,&final)||!regular(op->source,&first)||!identity(&source.meta,&first.meta)||
            !regular(op->target,&second)||!identity(&target.meta,&second.meta)||first.mount!=prior.base||second.mount!=prior.base) return refuse("provenance");
        int from=open(op->source,O_PATH|O_NOFOLLOW|O_CLOEXEC),to=open(op->target,O_PATH|O_NOFOLLOW|O_CLOEXEC);
        struct stat from_stat,to_stat;unsigned from_mount=0,to_mount=0;char from_path[64],to_path[64];
        int pinned=from>=0 && to>=0 && !fstat(from,&from_stat) && !fstat(to,&to_stat) &&
            identity(&from_stat,&source.meta) && identity(&to_stat,&target.meta) && mount_id(from,&from_mount) &&
            mount_id(to,&to_mount) && from_mount==now.base && to_mount==now.base &&
            build(from_path,sizeof(from_path),"/proc/self/fd/%d",from) && build(to_path,sizeof(to_path),"/proc/self/fd/%d",to);
        int result=pinned?mount(from_path,to_path,NULL,MS_BIND,NULL):-1;int error=errno;
        if(from>=0)close(from);
        if(to>=0)close(to);
        if(!pinned)return refuse("provenance");
        if(result)return kernel_refusal("mount_call",error);
        struct table next={0};struct intent seen={0};struct leaf visible;
        if(!current_views(op,&seen,&next,1)||!context(&now,&seen)||next.mounts!=1||next.parent!=now.base||next.top_major!=now.source_major||
            next.top_minor!=now.source_minor||strcmp(next.top_root,now.root)||!regular(op->target,&visible)||visible.mount!=next.top||
            !identity(&source.meta,&visible.meta))return refuse("mount_unconfirmed");
        if(!build(active,sizeof(active),"%s %u %" PRIu64 "\n",op->label,next.top,nonreusable_mount(op->target))||!record(op->authority,"bind-active",active))return refuse("mount_unconfirmed");
        *bound=1;return 1;
    }
    if(!strcmp(command,"status"))return 1;
    if(!strcmp(command,"absent"))return !*bound || refuse("unknown_state");
    if(!*bound)return 1;
    if(umount2(op->target,0))return kernel_refusal("unmount_call",errno);
    struct table next={0};struct intent seen={0};struct leaf revealed;
    if(!current_views(op,&seen,&next,1)||!context(&saved,&seen)||next.mounts||!regular(op->target,&revealed)||revealed.mount!=now.base||
        !identity(&revealed.meta,&saved.target)||!public_files(op,&source,&revealed,0,0))return refuse("unmount_unconfirmed");
    *bound=0;return 1;
}

int main(int argc,char **argv) {
    if(argc==2 && !strcmp(argv[1],"--policy")){puts("selector-bind-v1");return 0;}
    if(argc!=4 || (strcmp(argv[2],"stock")&&strcmp(argv[2],"scratch"))||strlen(argv[3])!=16||strspn(argv[3],"0123456789abcdef")!=16)return 64;
    const char *command=argv[1];
    if(strcmp(command,"prepare")&&strcmp(command,"bind")&&strcmp(command,"current")&&strcmp(command,"status")&&strcmp(command,"restore")&&strcmp(command,"absent"))return 64;
    struct operation op={.stock=!strcmp(argv[2],"stock"),.profile=argv[2],.label=argv[3],.authority=-1};int bound=0;
    if(!paths(&op)||!probe(&op,command,&bound)){fprintf(stderr,"refused bind_%s\n",reason);return 1;}
    puts(!strcmp(command,"prepare")?"bind_prepared":!strcmp(command,"restore")?"bind_restored":!strcmp(command,"absent")?"bind_absent":bound?"bound":"unbound");return 0;
}

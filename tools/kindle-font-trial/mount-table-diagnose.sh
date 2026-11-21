#!/bin/sh
set -fu
export LC_ALL=C
[ "$#" = 0 ] || exit 64
volume=/mnt/us
table=/proc/self/mountinfo
fdinfo=/proc/self/fdinfo/8
inspect_table() {
    awk -v table="$table" -v volume="$volume" -v layout="$1" -v fdinfo="$fdinfo" '
    function cardinal(value) { return value == 0 ? "zero" : value == 1 ? "one" : "multiple" }
    function positive(word) { return word ~ /^[0-9]+$/ && length(word)<=10 && word+0>0 && word+0<=4294967295 }
    function issue(code, area) { if (failure=="none") { failure=code; scope=area } }
    function io_fail(code) { table_status=code=="table_open" ? "open_error" : "read_error"; issue(code,"indeterminate") }
    function descriptor(     field,read_status,seen,value,invalid) {
        if (layout!="ok") return "dir_open"
        while ((read_status=(getline field < fdinfo))>0) if (field ~ /^mnt_id:/) {
            ++seen; value=substr(field,8); sub(/^[[:space:]]*/,"",value); sub(/[[:space:]]*$/,"",value)
            if (!positive(value)) invalid=1; else observed_fd=value+0
        }
        close(fdinfo)
        if (read_status<0) { observed_fd=0; return "fdinfo_read" }
        if (invalid || seen>1) { observed_fd=0; return "format" }
        return seen ? "ok" : "missing"
    }
    function parse(pass,     row,read_status,rows,count,field,area,id,parent,dev,root,point,delimiter,covered,shape_now,index_no) {
        while ((read_status=(getline row < "/proc/self/fd/9"))>0) {
            if (++rows>8192) { issue("row_limit","indeterminate"); complete=0; break }
            if (length(row)+1>32768) { issue("line_limit","indeterminate"); complete=0; break }
            count=split(row,field,/ +/); id=field[1]; parent=field[2]; dev=field[3]; root=field[4]; point=field[5]
            area=(point==volume ? "volume" : (count>=5 ? "other" : "indeterminate"))
            if (count<6) issue("missing_field",area)
            else if (!positive(id)) issue("mount_id",area)
            else if (!positive(parent)) issue("parent_id",area)
            else if (dev !~ /^[+]?[0-9]+:[+]?[0-9]+$/) issue("device_field",area)
            else if (substr(root,1,1)!="/") issue("root_nonabsolute",area)
            else if (substr(point,1,1)!="/") issue("point_nonabsolute",area)
            if (id+0==observed_fd && fd_status=="ok" && positive(id)) { ++fd_total; if (point==volume) ++base_fd }
            if (point==volume) {
                ++bases; delimiter=0; covered=0
                for (index_no=7;index_no<=count;++index_no) {
                    if (field[index_no]=="-") { delimiter=1; break }
                    if (field[index_no] ~ /^(shared:|master:|propagate_from:)/ || field[index_no]=="unbindable") covered=1
                }
                shape_now=(substr(root,1,1)!="/" ? "nonabsolute" : index(root,"\\") ? "escaped" : length(root)+length(suffix)>=4096 ? "oversize" : root=="/" ? "root" : "subdir")
                shape=(bases==1 ? shape_now : "ambiguous"); prop=(bases==1 && delimiter ? (covered ? "shared_or_slave" : "private") : "unknown")
                if (bases>1) issue("base_duplicate",area)
                else if (index(root,"\\")) issue("base_escaped",area)
                else if (shape_now=="oversize") issue("base_root_limit",area)
                else if (!delimiter) issue("base_separator",area)
                if (bases==1) { base=id+0; device=dev; base_root=root }
            }
            if (pass==2 && id+0==saved_base && saved_base && (point!=volume || dev!=saved_dev)) issue("base_changed",area)
            if (pass==2 && point==target) {
                if (length(root)>=4096) issue("target_root_limit",area)
                delimiter=0; for(index_no=7;index_no<=count;++index_no) if(field[index_no]=="-") delimiter=1
                if(!delimiter) issue("target_separator",area)
            }
        }
        if (read_status<0) { complete=0; io_fail("table_read") }
        close("/proc/self/fd/9")
        if (!bases && complete) issue("base_absent","indeterminate")
    }
    function reset() { failure="none";scope="none";shape="none";prop="unknown";bases=base_fd=fd_total=base=0;device=base_root="";complete=1 }
    function projection() { return failure SUBSEP scope SUBSEP shape SUBSEP prop SUBSEP bases SUBSEP base_fd SUBSEP fd_total SUBSEP base SUBSEP device SUBSEP base_root }
    function print_result() {
        printf "{\"policy\":\"mount-table-read-v1\",\"table\":\"%s\",\"fd\":\"%s\",\"base_rows\":\"%s\",\"base_fd_rows\":\"%s\",\"fd_rows\":\"%s\",\"base_shape\":\"%s\",\"first_error\":\"%s\",\"error_scope\":\"%s\",\"repeat\":\"%s\",\"seek\":\"not_observed\",\"propagation\":\"%s\"}\n",table_status,fd_status,cardinal(first_bases),cardinal(first_fd),cardinal(first_total),first_shape,first_error,first_scope,repeated,first_prop
    }
    BEGIN {
        suffix="/.weread-bind-diagnostic-3a578c34b99e748b/source"; target=volume "/.weread-bind-diagnostic-3a578c34b99e748b/target"
        table_status="ok";fd_status=descriptor(); reset(); parse(1); original=projection()
        first_error=failure;first_scope=scope;first_bases=bases;first_fd=base_fd;first_total=fd_total;first_shape=shape;first_prop=prop
        saved_base=base;saved_dev=device;repeated="unavailable"
        if (complete) {
            reset();parse(2)
            if (complete) repeated=(projection()==original ? "same" : "different")
            if (first_error=="none" && failure!="none") {first_error=failure;first_scope=scope}
        }
        print_result()
    }' 2>/dev/null
}
if ! ( : <"$table" ) 2>/dev/null; then
    printf '%s\n' '{"policy":"mount-table-read-v1","table":"open_error","fd":"not_observed","base_rows":"zero","base_fd_rows":"zero","fd_rows":"zero","base_shape":"none","first_error":"table_open","error_scope":"indeterminate","repeat":"unavailable","seek":"not_observed","propagation":"unknown"}'
elif ( : <"$volume" ) 2>/dev/null; then
    inspect_table ok 8<"$volume" 9<"$table"
else
    inspect_table dir_open 9<"$table"
fi

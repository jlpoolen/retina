#!/usr/bin/perl
#
# script to launch multiple sessions of retina against a list of cameras
# Prerequisites:
#   1) create identical accounts on all cameras
#   2) create a row, tab delimited, in the DATA section for each camera
#
# This is a base script to automate launching.  Eventual goal will be
# to have an auto-launcher, monitor the process, and register error events
# in a SQLite database for analysis and debugging
#
#
#

=pod
To monitor what's running:
   ps -efww |grep mp4

   ls -lath /tmp/retina

=cut


use strict;
use warnings;

use File::Path;

my $retina_root_dir = '/usr/local/src/retina';

print "Commencing $0 at ".localtime."\n";
#
# Move to the project directory so we can launch cargo from the
# relative point.
# 
chdir $retina_root_dir;
#
# use same credentials for consistency
#
my $user     = "retina";
my $password = "testingisfun";
#
# The working area
#
my $out_dir = "/tmp/retina";
mkdir -p $out_dir unless -d $out_dir;
chmod 0777, $out_dir;

while (my $data = <DATA>){
    chomp $data;
    my ($camera_type,$camera,$ip) = split("\t",$data);

    &create_test($camera_type,$camera,$ip);
}


#
# ------------------------ subs ----------------------
#
sub create_test {
    my ($camera_model, $camera, $ip) = @_;
    #
    # Unify file name and log with unique time stamp
    #
    $camera =~ s/\s/_/g;  # replace white spaces
    my $timestamp = `date +%Y%m%d_%H%M%S`;
    chomp $timestamp;
    my $log = "$out_dir/$camera\_$timestamp\.log";
    my $output = "$out_dir/$camera\_$timestamp\.mp4";
    
    my $rc = &start_stream($camera_model,$camera,
			   $ip,$user,$password,$output,$log);
    print "rc = $rc\n";
    
    print "Commenced $camera, output in $out_dir\n\n";
}


sub start_stream {
    my ($camera_model,$camera, $ip, $user, $password, $mp4_output, $log) = @_;
   
    my $url;
    #
    # For each brand/model of camers which probably has it's own
    # customized URL for rtsp access
    #
    if ($camera_model eq "Reolink"){
	$url = "rtsp://$ip:554/h264Preview_01_main";
    } else {
	die "Unknown camera type/model: $camera_model";
    }
    
    my $cmd = qq{cargo run --example client mp4 $mp4_output --url $url --username $user --password $password  >$log  2>&1 &};
    #
    # we'll fork since we're logging
    #
    print "About to launch:\n$cmd\n";
    system($cmd);
    return 1
}
#
# DATA format
# [camera type] tab [camera name] tab [network address: name or IP]
#
__DATA__
Reolink	Garage West	192.168.1.48
Reolink	Garage East	192.168.1.49
Reolink	Peck West Alley	192.168.1.52
Reolink	Peck East Alley	192.168.1.53

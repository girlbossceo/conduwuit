You will need a public IP address with a DNS name.

Be sure and set the user name and password in the Vagrentfile.
Make sure that there is no other VM named "arch" or edit the
 Vagrantfile to include setting the hostname.

The helper scripts:
    setup-arch-linux-libvirt
    setup-arch-linux-virtualbox
are there to help you install either KVM/QEMU/libvirt or VirtualBox.
Read those scripts and apply any missing configuration to you Computer.

To get the VM to launch, just run:
    vagrant up

Once the VM is running, login to it using the user name and password
 from the Vagrantfile which you should have modified like in the
 first instruction.

Once more than 25 minutes have passed, you can ssh into your VM:
    ssh vagrant@arch

Edit /etc/conduwuit.toml changing server_name to be
 your public DNS name.

Then just run:

sudo conduwuit -c /etc/conduwuit.toml

That's it, you will have a working conduit or raise an issue to:
 kstailey@proton.me
Please study how to write a decent bug report:
http://www.catb.org/~esr/faqs/smart-questions.html

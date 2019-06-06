pipeline {
  agent any

  stages {
    stage('Build') {
      steps {
        sh'''
          #!/bin/bash
          cd padre
          cargo build
          cd ..
          docker build -t vim-padre/test-container .
        '''
      }
    }

    stage('VIM Python 2 Unit Tests') {
      steps {
        sh'''
          #!/bin/bash
          cd pythonx
          python -m unittest discover -v test/
        '''
      }
    }

    stage('VIM Python 3 Unit Tests') {
      steps {
        sh'''
          #!/bin/bash
          cd pythonx
          python3 -m unittest discover -v test/
        '''
      }
    }

    stage('PADRE Unit Tests') {
      steps {
        sh'''
          #!/bin/bash
          cd padre
          cargo test -- --nocapture
        '''
      }
    }

    stage('PADRE Integration Tests') {
      steps {
        sh'''
          #!/bin/bash
          set +x
          . /var/lib/jenkins/behave/bin/activate
          cd padre/integration/
          behave
        '''
      }
    }

    stage('Vader Unit Tests') {
      steps {
        sh'''
          #!/bin/bash
          docker run -a stderr -e VADER_OUTPUT_FILE=/dev/stderr --rm vim-padre/test-container vim '+Vader! test/unit/*.vader' 2>&1
        '''
      }
    }

    stage('Vader Integration Tests') {
      steps {
        sh'''
          #!/bin/bash
          docker run -a stderr -e VADER_OUTPUT_FILE=/dev/stderr --privileged --rm vim-padre/test-container vim '+Vader! test/integration/*.vader' 2>&1
        '''
      }
    }
  }

// TODO:
//  post {
//    always {
//      step([
//        $class: 'hudson.plugins.robot.RobotPublisher',
//        outputPath: './',
//        passThreshold : 100,
//        unstableThreshold: 100,
//        otherFiles: '',
//        reportFileName: 'padre/integration/report*.html',
//        logFileName: 'padre/integration/log*.html',
//        outputFileName: 'padre/integration/output*.xml'
//      ])
//    }
//  }
}
